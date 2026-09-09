use crate::{
    context::{Checkpoint, CheckpointDraft, CHECKPOINT_SCHEMA_VERSION},
    model::{SessionEntry, SEMANTIC_EVENT_SCHEMA_VERSION},
    session_forest::{append_in_transaction, EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const CONTEXT_PRESSURE_PERCENT: f64 = 75.0;
pub const CHECKPOINT_TIMEOUT_SECONDS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    ContextPressure,
    ResponseReserve,
    PhaseBoundary,
    BeforeSuspend,
    BeforeDowngrade,
    BeforeShutdown,
    Manual,
}

impl CompactionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContextPressure => "context_pressure",
            Self::ResponseReserve => "response_reserve",
            Self::PhaseBoundary => "phase_boundary",
            Self::BeforeSuspend => "before_suspend",
            Self::BeforeDowngrade => "before_downgrade",
            Self::BeforeShutdown => "before_shutdown",
            Self::Manual => "manual",
        }
    }

    /// Why Bridge is asking, in the prompt's own voice. An agent that knows
    /// what the checkpoint is for can judge what belongs in the summary.
    const fn why_asked(self) -> &'static str {
        match self {
            Self::ContextPressure | Self::ResponseReserve => {
                "This session's context is nearly full, so I need a summary to carry forward before older turns are dropped."
            }
            Self::PhaseBoundary => {
                "This session has reached a phase boundary, so I need a summary to carry into the next phase."
            }
            Self::BeforeSuspend => {
                "This session is about to be suspended, so I need a summary to resume it from later."
            }
            Self::BeforeDowngrade => {
                "The user is switching this chat to a different model, so I need a summary for the incoming model to inherit."
            }
            Self::BeforeShutdown => {
                "This session is shutting down, so I need a summary of it kept on record."
            }
            Self::Manual => "Someone asked to compact this session, so I need a summary to carry forward.",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "context_pressure" => Self::ContextPressure,
            "response_reserve" => Self::ResponseReserve,
            "phase_boundary" => Self::PhaseBoundary,
            "before_suspend" | "warm_idle_timeout" => Self::BeforeSuspend,
            "before_downgrade" => Self::BeforeDowngrade,
            "before_shutdown" => Self::BeforeShutdown,
            "manual" => Self::Manual,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerState {
    pub reason: CompactionReason,
    pub context_percent: Option<f64>,
    pub projected_tokens_with_reserve: Option<i64>,
    pub context_window_tokens: Option<i64>,
    pub has_valid_typed_result: bool,
    pub one_shot_worker: bool,
    pub tool_call_active: bool,
    pub approval_active: bool,
    pub has_meaningful_new_work: bool,
    pub wall_clock_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionReason {
    OneShotResult,
    ToolCallActive,
    ApprovalActive,
    NoMeaningfulWork,
    WallClockOnly,
    BelowThreshold,
}

pub fn decide(state: &TriggerState) -> Result<CompactionReason, SuppressionReason> {
    if state.one_shot_worker && state.has_valid_typed_result {
        return Err(SuppressionReason::OneShotResult);
    }
    if state.tool_call_active {
        return Err(SuppressionReason::ToolCallActive);
    }
    if state.approval_active {
        return Err(SuppressionReason::ApprovalActive);
    }
    if !state.has_meaningful_new_work {
        return Err(SuppressionReason::NoMeaningfulWork);
    }
    if state.wall_clock_only {
        return Err(SuppressionReason::WallClockOnly);
    }
    let fires = match state.reason {
        CompactionReason::ContextPressure => state
            .context_percent
            .is_some_and(|percent| percent >= CONTEXT_PRESSURE_PERCENT),
        CompactionReason::ResponseReserve => match (
            state.projected_tokens_with_reserve,
            state.context_window_tokens,
        ) {
            (Some(projected), Some(window)) => projected >= window,
            _ => false,
        },
        CompactionReason::PhaseBoundary
        | CompactionReason::BeforeSuspend
        | CompactionReason::BeforeDowngrade
        | CompactionReason::BeforeShutdown
        | CompactionReason::Manual => true,
    };
    fires
        .then_some(state.reason)
        .ok_or(SuppressionReason::BelowThreshold)
}

/// What happened when a checkpoint was requested.
///
/// `begin` used to answer `Option<String>`, which meant "here is the prompt"
/// or "no prompt, work it out". Two callers cannot work it out: the warm-worker
/// reaper reads a bare `None` as "already pending" and leaves the worker warm,
/// reselecting it every maintenance tick forever; and manual compaction
/// reports "Compaction is already pending" for any refusal at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStart {
    /// Send this checkpoint prompt to the provider.
    Ready(String),
    /// A checkpoint is already in flight for this session.
    AlreadyPending,
    /// The provider is out of quota until `until`. It cannot summarise
    /// anything, and asking costs a turn to be told so again.
    ProviderLimited { harness: String, until: DateTime<Utc> },
}

impl CompactionStart {
    /// The prompt, for callers whose only question is whether to send one.
    pub fn prompt(self) -> Option<String> {
        match self {
            Self::Ready(prompt) => Some(prompt),
            _ => None,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCompaction {
    pub reason: CompactionReason,
    pub attempt: u8,
    pub tokens_before: i64,
    pub requested_at: String,
    pub first_retained_entry_id: String,
    /// Asked of a provider that no longer serves the session's conversation:
    /// a model switch's outgoing runtime, detached and summarising after the
    /// switch committed. The session's live reader must not treat the
    /// incoming model's turns as this request's reply, and the reply may land
    /// after the new model has already spoken.
    pub background: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionFailurePresentation {
    pub kind: &'static str,
    pub message: String,
    pub retryable: bool,
    pub recovery_action: String,
}

/// Convert implementation diagnostics into stable, safe UI copy while the
/// original reason remains available for the transcript inspector and audit
/// log. Classification is deliberately conservative: a failure never claims
/// history was lost, because compaction only moves the active boundary after a
/// fully verified checkpoint commits.
pub fn classify_failure(
    reason: &str,
    trigger: Option<CompactionReason>,
) -> CompactionFailurePresentation {
    let lower = reason.to_ascii_lowercase();
    let kind = if [
        "timed out",
        "without an assistant response",
        "adapter exited",
        "adapter exit",
        "connection closed",
        "connection reset",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
    {
        "timeout_or_exit"
    } else if [
        "could not start",
        "could not be delivered",
        "not running",
        "pipe is closed",
        "wait failed",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
    {
        "provider_unavailable"
    } else if [
        "invalid",
        "parse",
        "schema",
        "metadata does not match",
        "missing decision",
        "missing file",
        "durable evidence",
        "omits",
        "checkpoint response",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
    {
        "invalid_checkpoint"
    } else {
        "unknown"
    };
    let switching = trigger == Some(CompactionReason::BeforeDowngrade);
    let message = if switching {
        match kind {
            "invalid_checkpoint" => "The previous model returned a checkpoint Bridge could not verify. The model switch continued with stored conversation history.",
            _ => "The previous model did not finish the checkpoint. The model switch continued with stored conversation history.",
        }
    } else {
        match kind {
            "timeout_or_exit" => "The provider did not finish compaction. The original conversation history is intact.",
            "provider_unavailable" => "Compaction could not reach a ready provider. The original conversation history is intact.",
            "invalid_checkpoint" => "Bridge could not verify the provider's checkpoint, so no conversation history was replaced.",
            _ => "Compaction failed before a verified checkpoint was stored. The original conversation history is intact.",
        }
    };
    let recovery_action = if switching {
        "Retry compaction after the new model starts, or keep working; the original history is intact."
    } else {
        "Retry compaction when the provider is ready, or keep working with the original history."
    };
    CompactionFailurePresentation {
        kind,
        message: message.to_owned(),
        retryable: true,
        recovery_action: recovery_action.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckpointEvidence {
    decisions: BTreeSet<String>,
    files_touched: BTreeSet<String>,
}

impl CheckpointEvidence {
    fn from_active_history(db: &Connection, session_id: &str) -> Result<Self, BridgeError> {
        Self::collect(db, session_id, false)
    }

    /// The evidence the outgoing model could actually have seen: everything
    /// since the last compaction up to (not including) its own request. A
    /// background summary must not be rejected for omitting a decision or file
    /// the incoming model produced while the summary was being written.
    fn from_history_before_request(db: &Connection, session_id: &str) -> Result<Self, BridgeError> {
        Self::collect(db, session_id, true)
    }

    fn collect(db: &Connection, session_id: &str, stop_at_request: bool) -> Result<Self, BridgeError> {
        let branch = SessionForest::new(db)
            .active_branch(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let start = branch
            .iter()
            .rposition(|entry| entry.kind == "compaction")
            .map(|index| index + 1)
            .unwrap_or(0);
        let end = if stop_at_request {
            branch
                .iter()
                .rposition(|entry| {
                    entry.kind == "compaction.requested"
                        && entry.payload.get("attempt").and_then(Value::as_u64).unwrap_or(0) == 0
                })
                .unwrap_or(branch.len())
        } else {
            branch.len()
        };
        let mut evidence = Self {
            decisions: BTreeSet::new(),
            files_touched: BTreeSet::new(),
        };
        for entry in &branch[start..end.max(start)] {
            if matches!(entry.kind.as_str(), "worker.result" | "checkpoint") {
                collect_strings(&entry.payload, "decisions", &mut evidence.decisions);
            }
            if entry.kind == "worker.result" {
                collect_strings(&entry.payload, "filesChanged", &mut evidence.files_touched);
            } else if entry.kind == "artifact.created" {
                if let Some(path) = entry
                    .payload
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    evidence.files_touched.insert(path.to_owned());
                }
            }
        }
        Ok(evidence)
    }

    /// The evidence as a line the request can carry.
    ///
    /// Empty when there is nothing durable to account for, so a short session
    /// is not handed a list of nothing.
    fn prompt_section(&self) -> String {
        if self.decisions.is_empty() && self.files_touched.is_empty() {
            return String::new();
        }
        let mut parts = Vec::new();
        if !self.decisions.is_empty() {
            parts.push(format!(
                "decisions already on record: {}",
                self.decisions.iter().cloned().collect::<Vec<_>>().join("; ")
            ));
        }
        if !self.files_touched.is_empty() {
            parts.push(format!(
                "files already on record: {}",
                self.files_touched
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        format!(
            " Bridge has these on record from this session, so account for \
             them in your lists ({}).",
            parts.join(", ")
        )
    }

    /// Fill in what the checkpoint left out, and say who filled it.
    ///
    /// Replaces rejecting an otherwise good summary for an incomplete list.
    /// Bridge scanned the branch itself, so it already holds every missing
    /// item; spending a repair turn to be told them again, and then failing
    /// the boundary when the second reply also missed one, discarded a usable
    /// summary over bookkeeping Bridge could complete in place.
    ///
    /// Returns the provenance the boundary is stamped with: `agent` when the
    /// session accounted for its own evidence, `agent+controller` when Bridge
    /// had to add to it.
    fn augment(&self, checkpoint: &mut Checkpoint) -> &'static str {
        let missing_decisions = self.missing_from(&self.decisions, &checkpoint.decisions);
        let missing_files = self.missing_from(&self.files_touched, &checkpoint.files_touched);
        if missing_decisions.is_empty() && missing_files.is_empty() {
            return "agent";
        }
        checkpoint.decisions.extend(missing_decisions);
        checkpoint.files_touched.extend(missing_files);
        "agent+controller"
    }

    fn missing_from(&self, required: &BTreeSet<String>, present: &[String]) -> Vec<String> {
        let seen = present
            .iter()
            .map(|value| value.trim().to_owned())
            .collect::<BTreeSet<_>>();
        required.difference(&seen).cloned().collect()
    }

}

fn collect_strings(payload: &Value, field: &str, target: &mut BTreeSet<String>) {
    for value in payload
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let value = value.trim();
        if !value.is_empty() {
            target.insert(value.to_owned());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointOutcome {
    Completed {
        checkpoint_entry_id: String,
        compaction_entry_id: String,
    },
    /// A valid background summary that arrived after the incoming model had
    /// already spoken: recorded as a plain `checkpoint`, no boundary moved.
    LateCheckpoint {
        checkpoint_entry_id: String,
    },
    Repair {
        prompt: String,
    },
    Failed,
    NotPending,
}

pub struct CompactionController;

impl CompactionController {
    /// The maintenance turn Bridge sends its own agent.
    ///
    /// It names its asker and says why. The previous wording was a bare schema
    /// with "do no more work" and an unexplained `sourceAgent` the agent could
    /// not verify — which reads exactly like an injected instruction, and an
    /// agent that treats it as one is right to refuse. It also has to say that
    /// empty arrays are valid: told to fill `decisions` and `filesTouched` from
    /// a conversation where nothing was decided and nothing was touched, an
    /// honest agent's only options are to invent or to refuse.
    /// The request Bridge sends the session for its own checkpoint.
    ///
    /// Asks for meaning only. Every piece of bookkeeping the boundary needs
    /// (schema version, source agent, first retained entry, token count,
    /// reason) is filled in by Bridge after the reply lands, so a model can no
    /// longer fail a checkpoint by mistyping a UUID it was handed, and there
    /// is no request-matching value left for it to get wrong.
    ///
    /// `evidence` rides on the first attempt, not held back for a repair. The
    /// old prompt showed a model what it had missed only after rejecting it
    /// once, which spent a whole turn establishing something the request could
    /// have said up front.
    fn checkpoint_prompt(
        _session_id: &str,
        pending: &PendingCompaction,
        repair_error: Option<&str>,
        evidence: Option<&CheckpointEvidence>,
    ) -> String {
        let repair = repair_error
            .map(|error| {
                format!(" Your previous reply could not be read as that object: {error}. Send the object.")
            })
            .unwrap_or_default();
        let account_for = evidence
            .map(CheckpointEvidence::prompt_section)
            .unwrap_or_default();
        format!(
            "Bridge is asking, not the person you are talking to. {why} Reply \
             with one JSON object and nothing else: \
             {{\"summary\":\"what this session has been about\",\"decisions\":\
             [],\"filesTouched\":[],\"openWork\":[]}} `summary` is prose. \
             `decisions` are the choices made and worth keeping. \
             `filesTouched` are paths actually changed. `openWork` is what is \
             still unfinished. Report only what happened: leave a list empty \
             if there is nothing in it, and say so in `summary` if this \
             session has barely started. Invent nothing.{account_for} This is \
             bookkeeping, so do not call tools, run commands, change files, or \
             delegate; the only thing this turn may produce is that object. \
             Nothing you write here reaches the user and none of it becomes \
             part of your conversation with them.{repair}",
            why = pending.reason.why_asked(),
        )
    }

    pub fn begin(
        db: &Connection,
        session_id: &str,
        reason: CompactionReason,
        tokens_before: i64,
    ) -> Result<CompactionStart, BridgeError> {
        Self::begin_with_options(db, session_id, reason, tokens_before, false)
    }

    /// [`Self::begin`] for a request the session's *former* runtime will
    /// answer after a model switch has committed. See
    /// [`PendingCompaction::background`].
    pub fn begin_background(
        db: &Connection,
        session_id: &str,
        reason: CompactionReason,
        tokens_before: i64,
    ) -> Result<CompactionStart, BridgeError> {
        Self::begin_with_options(db, session_id, reason, tokens_before, true)
    }

    fn begin_with_options(
        db: &Connection,
        session_id: &str,
        reason: CompactionReason,
        tokens_before: i64,
        background: bool,
    ) -> Result<CompactionStart, BridgeError> {
        if Self::pending(db, session_id)?.is_some() {
            return Ok(CompactionStart::AlreadyPending);
        }
        // A compaction is a provider turn like any other, and an exhausted
        // provider fails it exactly as fast as it fails real work. Unguarded,
        // that produced 3,703 "Error running remote compact task: You've hit
        // your usage limit" turns against an account that had already said no.
        if let Some((harness, until)) =
            crate::learning_router::session_harness_cooldown(db, session_id)
        {
            let _ = crate::store::event(
                db,
                "compaction",
                "compaction.suppressed_provider_limit",
                session_id,
                &format!(
                    "{harness} is out of quota until {until}; compaction ({}) was not attempted",
                    reason.as_str()
                ),
            );
            return Ok(CompactionStart::ProviderLimited { harness, until });
        }
        let first_retained_entry_id = Uuid::new_v4().to_string();
        let pending = PendingCompaction {
            reason,
            attempt: 0,
            tokens_before: tokens_before.max(0),
            requested_at: Utc::now().to_rfc3339(),
            first_retained_entry_id,
            background,
        };
        SessionForest::new(db)
            .append(
                session_id,
                EntryKind::CompactionRequested,
                json!({
                    "reason": reason.as_str(),
                    "attempt": 0,
                    "tokensBefore": pending.tokens_before,
                    "sourceAgent": session_id,
                    "requestedAt": pending.requested_at,
                    "firstRetainedEntryId": pending.first_retained_entry_id,
                    "background": background,
                }),
            )
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        // The evidence rides on the *first* request. Holding it back for a
        // repair is what made the opening attempt guess at a list Bridge had
        // already scanned, and then spend a turn being corrected.
        let evidence = CheckpointEvidence::from_active_history(db, session_id)?;
        Ok(CompactionStart::Ready(Self::checkpoint_prompt(
            session_id,
            &pending,
            None,
            Some(&evidence),
        )))
    }

    /// Backward-compatible lifecycle hook used by issue #9. The production
    /// maintenance path consumes the returned prompt through `begin`.
    pub fn request_before_suspend(
        db: &Connection,
        session_id: &str,
        _legacy_reason: &str,
    ) -> Result<(), BridgeError> {
        let tokens = active_token_estimate(db, session_id)?;
        let _ = Self::begin(db, session_id, CompactionReason::BeforeSuspend, tokens)?;
        Ok(())
    }

    pub fn pending(
        db: &Connection,
        session_id: &str,
    ) -> Result<Option<PendingCompaction>, BridgeError> {
        let branch = SessionForest::new(db)
            .active_branch(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        pending_from_branch(&branch)
    }

    pub fn handle_output(
        db: &Connection,
        session_id: &str,
        output: &str,
    ) -> Result<CheckpointOutcome, BridgeError> {
        let Some(pending) = Self::pending(db, session_id)? else {
            return Ok(CheckpointOutcome::NotPending);
        };
        // The evidence Bridge scanned for itself. Read before the reply is
        // parsed because it is also what a repair request carries.
        let evidence = if pending.background {
            CheckpointEvidence::from_history_before_request(db, session_id)?
        } else {
            CheckpointEvidence::from_active_history(db, session_id)?
        };
        let draft = match CheckpointDraft::parse(output) {
            Ok(draft) => draft,
            Err(error) if pending.attempt == 0 => {
                return Self::request_repair(db, session_id, &pending, &error.to_string(), &evidence);
            }
            Err(error) => {
                Self::record_failure(db, session_id, &error.to_string(), pending.attempt)?;
                return Ok(CheckpointOutcome::Failed);
            }
        };
        // Bridge owns every field that identifies this boundary, so none of
        // them can arrive wrong. This is what retired the metadata-echo and
        // source-agent-mismatch failure kinds outright.
        let mut checkpoint = Checkpoint {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            summary: draft.summary,
            decisions: draft.decisions,
            files_touched: draft.files_touched,
            source_agent: session_id.to_owned(),
            first_retained_entry_id: pending.first_retained_entry_id.clone(),
            tokens_before: pending.tokens_before,
            reason: pending.reason.as_str().to_owned(),
            open_work: draft.open_work,
            provenance: None,
        };
        // An incomplete list is completed, not rejected: Bridge is holding the
        // missing items already, and the provenance says who supplied them.
        let provenance = evidence.augment(&mut checkpoint);
        if pending.background && conversation_appended_since_request(db, session_id)? {
            // The incoming model has already spoken. Moving the boundary now
            // would hide its turns behind a summary the outgoing model wrote
            // without seeing them, so the summary is kept as a plain
            // checkpoint the projection carries in its tail.
            return Self::record_late_checkpoint(db, session_id, checkpoint, pending, provenance);
        }
        Self::record_checkpoint(
            db,
            session_id,
            checkpoint,
            pending,
            provenance,
            Some(&evidence),
        )
    }

    /// The repair request on its own, for a caller that has already recorded
    /// the retry (the live-turn supervisor rebuilding a prompt after a
    /// timeout) and needs only the text.
    pub fn repair_prompt(
        db: &Connection,
        session_id: &str,
        pending: &PendingCompaction,
        error: &str,
    ) -> Result<String, BridgeError> {
        let evidence = CheckpointEvidence::from_active_history(db, session_id)?;
        Ok(Self::checkpoint_prompt(
            session_id,
            pending,
            Some(error),
            Some(&evidence),
        ))
    }

    /// Ask once more, saying what could not be read and what to account for.
    fn request_repair(
        db: &Connection,
        session_id: &str,
        pending: &PendingCompaction,
        error: &str,
        evidence: &CheckpointEvidence,
    ) -> Result<CheckpointOutcome, BridgeError> {
        SessionForest::new(db)
            .append(
                session_id,
                EntryKind::CompactionRequested,
                json!({
                    "reason": pending.reason.as_str(),
                    "attempt": 1,
                    "tokensBefore": pending.tokens_before,
                    "sourceAgent": session_id,
                    "requestedAt": Utc::now().to_rfc3339(),
                    "firstRetainedEntryId": pending.first_retained_entry_id,
                    "background": pending.background,
                    "repairOf": error,
                }),
            )
            .map_err(|forest_error| BridgeError::Invalid(forest_error.to_string()))?;
        Ok(CheckpointOutcome::Repair {
            prompt: Self::checkpoint_prompt(
                session_id,
                &PendingCompaction {
                    attempt: 1,
                    ..pending.clone()
                },
                Some(error),
                Some(evidence),
            ),
        })
    }


    /// Record a validated summary as a `checkpoint` entry only — no
    /// `compaction` boundary, no retained-set change. `pending_from_branch`
    /// treats a checkpoint newer than the request as its settlement.
    fn record_late_checkpoint(
        db: &Connection,
        session_id: &str,
        checkpoint: Checkpoint,
        pending: PendingCompaction,
        provenance: &str,
    ) -> Result<CheckpointOutcome, BridgeError> {
        let mut payload = checkpoint_payload(&checkpoint, &pending, provenance);
        payload["landing"] = json!("late");
        let entry = SessionForest::new(db)
            .append(session_id, EntryKind::Checkpoint, payload)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        store::event(
            db,
            "compaction",
            "checkpoint.landed_late",
            session_id,
            "Background handoff summary recorded as a checkpoint; the new model had already replied, so no boundary moved",
        )?;
        Ok(CheckpointOutcome::LateCheckpoint {
            checkpoint_entry_id: entry.id,
        })
    }

    pub fn record_reconstructed(
        db: &Connection,
        session_id: &str,
        summary: String,
        decisions: Vec<String>,
        files_touched: Vec<String>,
        reason: CompactionReason,
    ) -> Result<CheckpointOutcome, BridgeError> {
        let checkpoint = Checkpoint {
            schema_version: 1,
            summary,
            decisions,
            files_touched,
            source_agent: session_id.to_owned(),
            first_retained_entry_id: Uuid::new_v4().to_string(),
            tokens_before: active_token_estimate(db, session_id)?,
            reason: reason.as_str().to_owned(),
            // A reconstruction reads normalized events and Git facts, which
            // say what changed but never what is still unfinished.
            open_work: Vec::new(),
            provenance: Some("reconstructed".into()),
        };
        checkpoint
            .validate(Some(session_id))
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let pending = PendingCompaction {
            reason,
            attempt: 1,
            tokens_before: active_token_estimate(db, session_id)?,
            requested_at: Utc::now().to_rfc3339(),
            first_retained_entry_id: checkpoint.first_retained_entry_id.clone(),
            background: false,
        };
        Self::record_checkpoint(db, session_id, checkpoint, pending, "reconstructed", None)
    }

    pub fn reconstruct_from_normalized_events_and_git(
        db: &Connection,
        session_id: &str,
        git_status: &str,
    ) -> Result<CheckpointOutcome, BridgeError> {
        Self::reconstruct_from_normalized_events_and_git_with_reason(
            db,
            session_id,
            git_status,
            CompactionReason::PhaseBoundary,
        )
    }

    /// [`Self::reconstruct_from_normalized_events_and_git`] recording the
    /// reason the failed checkpoint was asked for, so a model switch whose
    /// background summary failed still leaves a `before_downgrade` checkpoint
    /// with `provenance: reconstructed` rather than nothing.
    pub fn reconstruct_from_normalized_events_and_git_with_reason(
        db: &Connection,
        session_id: &str,
        git_status: &str,
        reason: CompactionReason,
    ) -> Result<CheckpointOutcome, BridgeError> {
        let branch = SessionForest::new(db)
            .active_branch(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let mut facts = Vec::new();
        let mut decisions = Vec::new();
        for entry in branch.iter().rev() {
            match entry.kind.as_str() {
                "user.message" | "assistant.message" | "worker.result" => {
                    if let Some(summary) = entry
                        .payload
                        .get("summary")
                        .or_else(|| entry.payload.get("text"))
                        .or_else(|| entry.payload.pointer("/data/text"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        facts.push(summary.to_owned());
                    }
                    if let Some(values) = entry.payload.get("decisions").and_then(Value::as_array) {
                        for decision in values.iter().filter_map(Value::as_str) {
                            let decision = decision.trim();
                            if !decision.is_empty()
                                && !decisions.iter().any(|known| known == decision)
                            {
                                decisions.push(decision.to_owned());
                            }
                        }
                    }
                }
                _ => {}
            }
            if facts.len() >= 12 {
                break;
            }
        }
        facts.reverse();
        let files_touched = git_status
            .lines()
            .filter_map(|line| line.get(3..).map(str::trim))
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let summary = if facts.is_empty() {
            "Recovered checkpoint from immutable normalized events and current Git facts".into()
        } else {
            format!("Recovered from normalized events: {}", facts.join(" | "))
        };
        let outcome = Self::record_reconstructed(
            db,
            session_id,
            summary,
            decisions,
            files_touched,
            reason,
        )?;
        store::event(
            db,
            "compaction",
            "compaction.recovery_worker.completed",
            session_id,
            "Reconstructed from normalized events and Git facts",
        )?;
        Ok(outcome)
    }

    pub fn record_failure(
        db: &Connection,
        session_id: &str,
        reason: &str,
        attempt: u8,
    ) -> Result<(), BridgeError> {
        let trigger = Self::pending(db, session_id)?.map(|pending| pending.reason);
        let presentation = classify_failure(reason, trigger);
        SessionForest::new(db)
            .append(
                session_id,
                EntryKind::CompactionFailed,
                json!({
                    "reason": reason,
                    "attempt": attempt,
                    "failedAt": Utc::now().to_rfc3339(),
                    "failureKind": presentation.kind,
                    "message": presentation.message,
                    "retryable": presentation.retryable,
                    "recoveryAction": presentation.recovery_action,
                    "trigger": trigger.map(CompactionReason::as_str),
                }),
            )
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        store::event(db, "compaction", "compaction.failed", session_id, reason)?;
        Ok(())
    }


    fn record_checkpoint(
        db: &Connection,
        session_id: &str,
        checkpoint: Checkpoint,
        pending: PendingCompaction,
        provenance: &str,
        evidence: Option<&CheckpointEvidence>,
    ) -> Result<CheckpointOutcome, BridgeError> {
        if checkpoint.first_retained_entry_id != pending.first_retained_entry_id
            || checkpoint.tokens_before != pending.tokens_before
            || checkpoint.reason != pending.reason.as_str()
        {
            return Err(BridgeError::Invalid(
                "checkpoint metadata does not match its controller request".into(),
            ));
        }
        let transaction = db.unchecked_transaction()?;
        let checkpoint_payload = checkpoint_payload(&checkpoint, &pending, provenance);
        let checkpoint_entry = append_in_transaction(
            &transaction,
            session_id,
            EntryKind::Checkpoint,
            checkpoint_payload,
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let compaction_entry = append_in_transaction(
            &transaction,
            session_id,
            EntryKind::Compaction,
            json!({
                "schemaVersion": 1,
                "summary": checkpoint.summary,
                "decisions": checkpoint.decisions,
                "filesTouched": checkpoint.files_touched,
                "openWork": checkpoint.open_work,
                "firstRetainedEntryId": checkpoint.first_retained_entry_id,
                "tokensBefore": pending.tokens_before,
                "reason": pending.reason.as_str(),
                "sourceAgent": checkpoint.source_agent,
                "provenance": checkpoint.provenance.as_deref().unwrap_or(provenance),
            }),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let retained_at = Utc::now().to_rfc3339();
        let repository_state = store::repository_state_for_session(&transaction, session_id)?;
        let retained_sequence = transaction.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM session_entries WHERE session_id=?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )?;
        transaction.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,context_visibility,created_at)
             VALUES(?1,?2,?3,?4,?5,'branch.summary',?6,'eligible',?7)",
            params![
                checkpoint.first_retained_entry_id,
                session_id,
                compaction_entry.id,
                retained_sequence,
                SEMANTIC_EVENT_SCHEMA_VERSION,
                json!({
                    "summary": "Compaction boundary; subsequent entries are retained",
                    "_bridgeTypedSchemaVersion": 1,
                    "_bridgeRepoState": repository_state,
                })
                .to_string(),
                retained_at,
            ],
        )?;
        transaction.execute(
            "UPDATE session_heads SET active_entry_id=?2,latest_checkpoint_entry_id=?3,updated_at=?4 WHERE session_id=?1",
            params![
                session_id,
                checkpoint.first_retained_entry_id,
                checkpoint_entry.id,
                retained_at,
            ],
        )?;
        if let Some(evidence) = evidence {
            transaction.execute(
                "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('compaction','checkpoint.evidence_verified',?1,?2,?3)",
                params![
                    session_id,
                    json!({
                        "decisionCount": evidence.decisions.len(),
                        "fileCount": evidence.files_touched.len(),
                    }).to_string(),
                    retained_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(CheckpointOutcome::Completed {
            checkpoint_entry_id: checkpoint_entry.id,
            compaction_entry_id: compaction_entry.id,
        })
    }
}

fn checkpoint_payload(
    checkpoint: &Checkpoint,
    pending: &PendingCompaction,
    provenance: &str,
) -> Value {
    json!({
        "schemaVersion": checkpoint.schema_version,
        "summary": checkpoint.summary,
        "decisions": checkpoint.decisions,
        "filesTouched": checkpoint.files_touched,
        "openWork": checkpoint.open_work,
        "tokensBefore": pending.tokens_before,
        "reason": pending.reason.as_str(),
        "sourceAgent": checkpoint.source_agent,
        "firstRetainedEntryId": checkpoint.first_retained_entry_id,
        "provenance": checkpoint.provenance.as_deref().unwrap_or(provenance),
    })
}

fn pending_from_branch(branch: &[SessionEntry]) -> Result<Option<PendingCompaction>, BridgeError> {
    for entry in branch.iter().rev() {
        match entry.kind.as_str() {
            // A `checkpoint` is only ever appended as a request's answer —
            // paired with a `compaction` by `record_checkpoint`, or alone by a
            // late background landing — so one newer than the request settles it.
            "compaction" | "compaction.failed" | "checkpoint" => return Ok(None),
            "compaction.requested" => {
                let reason = entry
                    .payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .and_then(CompactionReason::parse)
                    .ok_or_else(|| BridgeError::Invalid("invalid compaction reason".into()))?;
                return Ok(Some(PendingCompaction {
                    reason,
                    attempt: entry
                        .payload
                        .get("attempt")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u8,
                    tokens_before: entry
                        .payload
                        .get("tokensBefore")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    requested_at: entry
                        .payload
                        .get("requestedAt")
                        .and_then(Value::as_str)
                        .unwrap_or(&entry.created_at)
                        .to_owned(),
                    first_retained_entry_id: entry
                        .payload
                        .get("firstRetainedEntryId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    background: entry
                        .payload
                        .get("background")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Whether any conversation entry landed after the pending request was first
/// made (its attempt-0 `compaction.requested`). Only a background request can
/// see this: a foreground checkpoint turn suppresses every conversation frame
/// until it settles.
pub fn conversation_appended_since_request(
    db: &Connection,
    session_id: &str,
) -> Result<bool, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let mut spoke = false;
    for entry in branch.iter().rev() {
        if entry.kind == "compaction.requested"
            && entry.payload.get("attempt").and_then(Value::as_u64).unwrap_or(0) == 0
        {
            return Ok(spoke);
        }
        if CONVERSATION_KINDS.contains(&entry.kind.as_str()) {
            spoke = true;
        }
    }
    Ok(false)
}

pub fn active_token_estimate(db: &Connection, session_id: &str) -> Result<i64, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(branch.iter().map(entry_token_estimate).sum())
}

/// The kinds that are the conversation itself, as opposed to the machinery
/// around it. The same set `plan_switch_summary`'s "meaningful work" gate
/// reads, kept in one place so the gate and the floor cannot drift.
pub const CONVERSATION_KINDS: [&str; 4] = [
    "user.message",
    "assistant.message",
    "worker.result",
    "tool.completed",
];

/// [`active_token_estimate`] restricted to [`CONVERSATION_KINDS`].
///
/// The full-branch estimate answers "how full is this context?" — it counts
/// injected instructions, lifecycle entries, everything — and a fresh chat
/// with a compiled prompt measures thousands of tokens before anyone says a
/// word. A floor on *conversation size* has to count only the conversation,
/// or a greeting clears it on the strength of context it did not write.
pub fn conversation_token_estimate(
    db: &Connection,
    session_id: &str,
) -> Result<i64, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(branch
        .iter()
        .filter(|entry| CONVERSATION_KINDS.contains(&entry.kind.as_str()))
        .map(entry_token_estimate)
        .sum())
}

fn entry_token_estimate(entry: &SessionEntry) -> i64 {
    entry.token_estimate.unwrap_or_else(|| {
        let bytes = entry.payload.to_string().len() as i64;
        (bytes + 3) / 4
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::ContextProjector, store};

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/context','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Pune','Context','bridge/context','/tmp/context-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Orchestrator','ready','reported')", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('s','fresh','now')", []).unwrap();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::UserMessage,
                json!({"text":"Keep the durable decision"}),
            )
            .unwrap();
        db
    }

    fn valid_output(pending: &PendingCompaction, summary: &str) -> String {
        json!({
            "schemaVersion": 1,
            "summary": summary,
            "decisions": ["Keep SQLite as source of truth"],
            "filesTouched": ["src-tauri/src/context.rs"],
            "sourceAgent": "s",
            "firstRetainedEntryId": pending.first_retained_entry_id,
            "tokensBefore": pending.tokens_before,
            "reason": pending.reason.as_str(),
        })
        .to_string()
    }

    fn state(reason: CompactionReason) -> TriggerState {
        TriggerState {
            reason,
            context_percent: Some(75.0),
            projected_tokens_with_reserve: Some(100),
            context_window_tokens: Some(100),
            has_valid_typed_result: false,
            one_shot_worker: false,
            tool_call_active: false,
            approval_active: false,
            has_meaningful_new_work: true,
            wall_clock_only: false,
        }
    }

    fn pending_for(reason: CompactionReason) -> PendingCompaction {
        PendingCompaction {
            reason,
            attempt: 0,
            tokens_before: 4_200,
            requested_at: "now".into(),
            first_retained_entry_id: "retained-1".into(),
            background: false,
        }
    }

    /// The wording is load-bearing. An agent that cannot tell who is asking,
    /// or that is told to fill arrays from a session where nothing happened,
    /// is right to refuse, and a refusal is what the user ends up looking at.
    #[test]
    fn the_checkpoint_prompt_asks_for_meaning_and_nothing_else() {
        let pending = pending_for(CompactionReason::BeforeDowngrade);
        let prompt = CompactionController::checkpoint_prompt("s", &pending, None, None);
        assert!(
            prompt.starts_with("Bridge is asking, not the person you are talking to."),
            "the prompt must say who is asking: {prompt}"
        );
        assert!(
            prompt.contains("switching this chat to a different model"),
            "and why it is asking: {prompt}"
        );
        assert!(
            prompt.contains("leave a list empty") && prompt.contains("Invent nothing"),
            "and that an empty answer is a valid one: {prompt}"
        );
        for forbidden_action in [
            "do not call tools",
            "run commands",
            "change files",
            "delegate",
            "the only thing this turn may produce",
        ] {
            assert!(
                prompt.contains(forbidden_action),
                "the maintenance turn must forbid {forbidden_action:?}: {prompt}"
            );
        }
        assert!(
            prompt.contains("reaches the user") && prompt.contains("part of your conversation"),
            "and that this turn is not the conversation: {prompt}"
        );
        // The mangled-continuation trap: a `\`-joined Rust literal that lost
        // its continuations reads as one line with runs of indentation in it.
        assert!(!prompt.contains("   "), "the prompt carries stray indentation: {prompt}");

        // The point of the rewrite: no bookkeeping is asked of the model, so
        // none of it can come back wrong. Bridge fills every one of these.
        for bookkeeping in [
            "retained-1",
            "4200",
            "before_downgrade",
            "sourceAgent",
            "firstRetainedEntryId",
            "tokensBefore",
            "schemaVersion",
        ] {
            assert!(
                !prompt.contains(bookkeeping),
                "the model is never asked for {bookkeeping:?}: {prompt}"
            );
        }
        // What it is asked for, and only that.
        for wanted in ["summary", "decisions", "filesTouched", "openWork"] {
            assert!(prompt.contains(wanted), "the prompt must ask for {wanted:?}: {prompt}");
        }
    }

    #[test]
    fn the_first_request_already_carries_the_evidence_to_account_for() {
        // The old prompt showed a model what it had missed only after
        // rejecting it once, spending a whole turn to establish something the
        // request could have said up front.
        let db = database();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::WorkerResult,
                json!({
                    "status":"completed",
                    "summary":"implemented",
                    "decisions":["Keep the public API"],
                    "filesChanged":["src/api.rs"]
                }),
            )
            .unwrap();
        let prompt = CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        assert!(prompt.contains("Keep the public API"), "{prompt}");
        assert!(prompt.contains("src/api.rs"), "{prompt}");
        assert!(prompt.contains("account for"), "{prompt}");
    }

    #[test]
    fn a_session_with_nothing_on_record_is_not_handed_a_list_of_nothing() {
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"just talking"}))
            .unwrap();
        let prompt = CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        assert!(!prompt.contains("already on record"), "{prompt}");
    }

    #[test]
    fn every_reason_explains_itself_in_the_prompt() {
        for reason in [
            CompactionReason::ContextPressure,
            CompactionReason::ResponseReserve,
            CompactionReason::PhaseBoundary,
            CompactionReason::BeforeSuspend,
            CompactionReason::BeforeDowngrade,
            CompactionReason::BeforeShutdown,
            CompactionReason::Manual,
        ] {
            let why = reason.why_asked();
            assert!(
                why.len() > 30 && why.ends_with('.'),
                "{reason:?} has no sentence explaining itself: {why}"
            );
            assert!(
                CompactionController::checkpoint_prompt("s", &pending_for(reason), None, None).contains(why),
                "{reason:?} does not carry its explanation into the prompt"
            );
        }
    }

    /// The prompt shows the object it wants back, so that object has to be one
    /// the reader accepts, including with the empty lists it explicitly
    /// permits for a session where nothing happened yet.
    #[test]
    fn an_empty_but_honest_checkpoint_commits() {
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"hello"}))
            .unwrap();
        CompactionController::begin(&db, "s", CompactionReason::BeforeDowngrade, 42)
            .unwrap().prompt()
            .unwrap();
        let output = json!({
            "summary": "This session had only just started; nothing was decided or changed.",
            "decisions": [],
            "filesTouched": [],
            "openWork": [],
        })
        .to_string();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", &output).unwrap(),
            CheckpointOutcome::Completed { .. }
        ));
        let boundary = store::session_entries(&db, "s")
            .unwrap()
            .into_iter()
            .find(|entry| entry.kind == "compaction")
            .expect("an honestly empty checkpoint is still a checkpoint");
        assert_eq!(boundary.payload["decisions"].as_array().unwrap().len(), 0);
        assert_eq!(boundary.payload["filesTouched"].as_array().unwrap().len(), 0);
        assert_eq!(boundary.payload["provenance"], "agent");
    }

    /// The two estimates answer different questions and must be allowed to
    /// disagree: the branch estimate counts everything context carries, the
    /// conversation estimate only what was said and done.
    #[test]
    fn conversation_estimate_ignores_machine_entries_the_branch_counts() {
        let db = database();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::SessionStatus,
                json!({"status": "ready", "detail": "x".repeat(8_000)}),
            )
            .unwrap();
        let branch = active_token_estimate(&db, "s").unwrap();
        let conversation = conversation_token_estimate(&db, "s").unwrap();
        assert!(
            branch > conversation + 1_500,
            "the machine entry counts toward the branch ({branch}) but not the conversation ({conversation})"
        );
        // The fixture's one user message is conversation, so the filtered
        // estimate is not simply zero.
        assert!(conversation > 0);
    }

    #[test]
    fn trigger_matrix_covers_every_reason_and_suppression() {
        for reason in [
            CompactionReason::ContextPressure,
            CompactionReason::ResponseReserve,
            CompactionReason::PhaseBoundary,
            CompactionReason::BeforeSuspend,
            CompactionReason::BeforeDowngrade,
            CompactionReason::BeforeShutdown,
            CompactionReason::Manual,
        ] {
            assert_eq!(decide(&state(reason)), Ok(reason));
        }
        let mut input = state(CompactionReason::ContextPressure);
        input.context_percent = Some(74.99);
        assert_eq!(decide(&input), Err(SuppressionReason::BelowThreshold));
        input.context_percent = Some(90.0);
        input.one_shot_worker = true;
        input.has_valid_typed_result = true;
        assert_eq!(decide(&input), Err(SuppressionReason::OneShotResult));
        input.one_shot_worker = false;
        input.tool_call_active = true;
        assert_eq!(decide(&input), Err(SuppressionReason::ToolCallActive));
        input.tool_call_active = false;
        input.approval_active = true;
        assert_eq!(decide(&input), Err(SuppressionReason::ApprovalActive));
        input.approval_active = false;
        input.has_meaningful_new_work = false;
        assert_eq!(decide(&input), Err(SuppressionReason::NoMeaningfulWork));
        input.has_meaningful_new_work = true;
        input.wall_clock_only = true;
        assert_eq!(decide(&input), Err(SuppressionReason::WallClockOnly));
    }

    #[test]
    fn invalid_checkpoint_gets_one_repair_then_records_failure_without_losing_raw_events() {
        let db = database();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", "not json").unwrap(),
            CheckpointOutcome::Repair { .. }
        ));
        assert_eq!(
            CompactionController::pending(&db, "s")
                .unwrap()
                .unwrap()
                .attempt,
            1
        );
        assert_eq!(
            CompactionController::handle_output(&db, "s", "still invalid").unwrap(),
            CheckpointOutcome::Failed
        );
        let entries = store::session_entries(&db, "s").unwrap();
        assert_eq!(entries[0].kind, "user.message");
        assert_eq!(entries[0].payload["text"], "Keep the durable decision");
        assert_eq!(entries.last().unwrap().kind, "compaction.failed");
        assert_eq!(entries.last().unwrap().payload["failureKind"], "invalid_checkpoint");
        assert_eq!(entries.last().unwrap().payload["retryable"], true);
        assert!(entries.last().unwrap().payload["message"]
            .as_str()
            .unwrap()
            .contains("could not verify"));
        assert!(CompactionController::pending(&db, "s").unwrap().is_none());
    }

    #[test]
    fn compaction_failure_classification_explains_retry_and_model_switch_fallback() {
        let timeout = classify_failure(
            "checkpoint turn completed without an assistant response",
            Some(CompactionReason::Manual),
        );
        assert_eq!(timeout.kind, "timeout_or_exit");
        assert!(timeout.retryable);
        assert!(timeout.message.contains("history is intact"));

        let unavailable = classify_failure(
            "checkpoint turn could not start: provider pipe is closed",
            Some(CompactionReason::Manual),
        );
        assert_eq!(unavailable.kind, "provider_unavailable");
        assert!(unavailable.recovery_action.contains("Retry compaction"));

        let switching = classify_failure(
            "checkpoint metadata does not match its controller request",
            Some(CompactionReason::BeforeDowngrade),
        );
        assert_eq!(switching.kind, "invalid_checkpoint");
        assert!(switching.message.contains("model switch continued"));
        assert!(switching.recovery_action.contains("new model"));

        let evidence = classify_failure(
            "checkpoint omits durable evidence (decisions: Keep the API)",
            Some(CompactionReason::ContextPressure),
        );
        assert_eq!(evidence.kind, "invalid_checkpoint");
        assert!(evidence.message.contains("could not verify"));

        let unknown = classify_failure("unexpected controller failure", None);
        assert_eq!(unknown.kind, "unknown");
        assert!(unknown.message.contains("original conversation history is intact"));
    }

    #[test]
    fn background_requests_are_parsed_and_only_a_matching_checkpoint_settles_them() {
        let db = database();
        // A checkpoint OLDER than the request is history, not an answer.
        SessionForest::new(&db)
            .append("s", EntryKind::Checkpoint, json!({"schemaVersion":1,"summary":"earlier"}))
            .unwrap();
        CompactionController::begin_background(&db, "s", CompactionReason::BeforeDowngrade, 42)
            .unwrap().prompt()
            .expect("nothing pending");
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        assert!(pending.background);
        assert_eq!(pending.reason, CompactionReason::BeforeDowngrade);
        // A foreground request is refused while the background one is pending,
        // and does not silently turn into a foreground parse of it.
        assert!(CompactionController::begin(&db, "s", CompactionReason::Manual, 1).unwrap().prompt().is_none());
        // A repair re-request keeps the flag.
        assert!(matches!(
            CompactionController::handle_output(&db, "s", "not json").unwrap(),
            CheckpointOutcome::Repair { .. }
        ));
        let _ = &pending;
        assert!(CompactionController::pending(&db, "s").unwrap().unwrap().background);
        // A checkpoint NEWER than the request settles it.
        SessionForest::new(&db)
            .append("s", EntryKind::Checkpoint, json!({"schemaVersion":1,"summary":"the answer"}))
            .unwrap();
        assert!(CompactionController::pending(&db, "s").unwrap().is_none());
        // Foreground requests default to not-background.
        CompactionController::begin(&db, "s", CompactionReason::Manual, 1).unwrap().prompt().unwrap();
        assert!(!CompactionController::pending(&db, "s").unwrap().unwrap().background);
    }

    #[test]
    fn late_landing_records_a_checkpoint_without_moving_the_boundary() {
        let db = database();
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed","summary":"verified",
            "decisions":["Keep SQLite as source of truth"],
            "filesChanged":["src-tauri/src/context.rs"]
        })).unwrap();
        CompactionController::begin_background(&db, "s", CompactionReason::BeforeDowngrade, 55)
            .unwrap().prompt()
            .unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        assert!(!conversation_appended_since_request(&db, "s").unwrap());
        // The incoming model speaks before the outgoing model's summary lands.
        SessionForest::new(&db)
            .append("s", EntryKind::UserMessage, json!({"text":"hello new model"}))
            .unwrap();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"hello, continuing"}))
            .unwrap();
        assert!(conversation_appended_since_request(&db, "s").unwrap());
        let outcome = CompactionController::handle_output(
            &db,
            "s",
            &valid_output(&pending, "What the old model knew"),
        )
        .unwrap();
        assert!(matches!(outcome, CheckpointOutcome::LateCheckpoint { .. }), "{outcome:?}");
        let entries = store::session_entries(&db, "s").unwrap();
        let kinds = entries.iter().map(|entry| entry.kind.as_str()).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["user.message", "worker.result", "compaction.requested", "user.message", "assistant.message", "checkpoint"]
        );
        let late = entries.last().unwrap();
        assert_eq!(late.payload["landing"], "late");
        assert_eq!(late.payload["provenance"], "agent");
        assert_eq!(late.payload["reason"], "before_downgrade");
        assert!(CompactionController::pending(&db, "s").unwrap().is_none(), "the request is settled");
        // No boundary moved: the new model's turns stay in the projection and
        // there is no restoration header hiding them. The late checkpoint is a
        // durable record, not re-injected conversation, so the projector's tail
        // does not carry it (bare checkpoints never render as conversation).
        let branch = SessionForest::new(&db).active_branch("s").unwrap();
        let projection = crate::context::ContextProjector::project(&branch, 128_000).unwrap();
        assert!(projection.restoration_context.is_none());
        assert!(projection.render_entries.iter().any(|entry| entry.kind == "assistant.message"));
        assert!(projection.render_entries.iter().all(|entry| entry.kind != "checkpoint"));
    }

    #[test]
    fn background_evidence_is_scoped_to_what_the_outgoing_model_saw() {
        let db = database();
        CompactionController::begin_background(&db, "s", CompactionReason::BeforeDowngrade, 9)
            .unwrap().prompt()
            .unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        // The incoming model lands a decision the outgoing model never saw.
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed","summary":"new work","decisions":["Adopt the new API"],
            "filesChanged":["src/new.rs"]
        })).unwrap();
        let output = json!({
            "schemaVersion":1,"summary":"the old conversation","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":pending.first_retained_entry_id,
            "tokensBefore":pending.tokens_before,"reason":pending.reason.as_str()
        }).to_string();
        let outcome = CompactionController::handle_output(&db, "s", &output).unwrap();
        assert!(
            matches!(outcome, CheckpointOutcome::LateCheckpoint { .. }),
            "evidence the outgoing model could not have seen must not reject its summary: {outcome:?}"
        );
    }

    #[test]
    fn a_foreground_request_never_lands_late_even_with_a_stale_conversation_flag() {
        // A foreground checkpoint turn suppresses conversation frames, so the
        // late rule is gated on `background`; a foreground request that somehow
        // sees a newer user message still commits the boundary.
        let db = database();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 7).unwrap().prompt().unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        SessionForest::new(&db)
            .append("s", EntryKind::UserMessage, json!({"text":"racing frame"}))
            .unwrap();
        let output = json!({
            "schemaVersion":1,"summary":"done","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":pending.first_retained_entry_id,
            "tokensBefore":pending.tokens_before,"reason":pending.reason.as_str()
        }).to_string();
        let outcome = CompactionController::handle_output(&db, "s", &output).unwrap();
        assert!(matches!(outcome, CheckpointOutcome::Completed { .. }), "{outcome:?}");
    }

    #[test]
    fn reconstruct_with_reason_records_before_downgrade() {
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"we chose the SQLite token store"}))
            .unwrap();
        let outcome = CompactionController::reconstruct_from_normalized_events_and_git_with_reason(
            &db,
            "s",
            " M src/store.rs\n",
            CompactionReason::BeforeDowngrade,
        )
        .unwrap();
        assert!(matches!(outcome, CheckpointOutcome::Completed { .. }));
        let entries = store::session_entries(&db, "s").unwrap();
        let checkpoint = entries.iter().find(|entry| entry.kind == "checkpoint").unwrap();
        assert_eq!(checkpoint.payload["reason"], "before_downgrade");
        assert_eq!(checkpoint.payload["provenance"], "reconstructed");
        assert!(checkpoint.payload["summary"].as_str().unwrap().contains("SQLite token store"));
        assert_eq!(checkpoint.payload["filesTouched"], json!(["src/store.rs"]));

        // The un-suffixed entry point keeps its phase-boundary reason.
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"phase work"}))
            .unwrap();
        CompactionController::reconstruct_from_normalized_events_and_git(&db, "s", "").unwrap();
        let entries = store::session_entries(&db, "s").unwrap();
        let checkpoint = entries.iter().find(|entry| entry.kind == "checkpoint").unwrap();
        assert_eq!(checkpoint.payload["reason"], "phase_boundary");
    }

    #[test]
    fn an_evidence_gap_is_completed_by_bridge_not_rejected() {
        // Bridge scanned the branch itself, so it is already holding every
        // item a checkpoint left out. Spending a repair turn to be told them
        // again, then failing the boundary when the second reply also missed
        // one, threw away a usable summary over bookkeeping.
        let db = database();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::WorkerResult,
                json!({
                    "status":"completed",
                    "summary":"implemented",
                    "decisions":["Keep the public API", "Keep the public API"],
                    "filesChanged":["src/api.rs", "src/api.rs"]
                }),
            )
            .unwrap();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::ArtifactCreated,
                json!({"path":"docs/api.md"}),
            )
            .unwrap();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        let incomplete = json!({
            "summary":"looks complete","decisions":[],"filesTouched":[]
        })
        .to_string();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", &incomplete).unwrap(),
            CheckpointOutcome::Completed { .. }
        ));

        let entries = store::session_entries(&db, "s").unwrap();
        let boundary = entries
            .iter()
            .find(|entry| entry.kind == "compaction")
            .expect("the boundary commits");
        assert_eq!(boundary.payload["summary"], "looks complete");
        assert_eq!(
            boundary.payload["provenance"], "agent+controller",
            "the record says which parts Bridge supplied"
        );
        let decisions = boundary.payload["decisions"].as_array().unwrap();
        let files = boundary.payload["filesTouched"].as_array().unwrap();
        assert_eq!(decisions, &[json!("Keep the public API")]);
        assert_eq!(
            files,
            &[json!("docs/api.md"), json!("src/api.rs")],
            "every durable file is present, and a duplicate on record is one entry"
        );
        assert!(
            !entries.iter().any(|entry| entry.kind == "compaction.failed"),
            "a completable gap is not a failure"
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.payload.get("repairOf").is_some()),
            "and it costs no repair turn"
        );

        // A stored boundary still has to satisfy the strict schema on the way
        // back out, duplicate rule included. Augmenting in place is only safe
        // because the draft is trimmed and deduplicated first, so this is the
        // assertion that would catch it if that ever stopped being true.
        let stored = entries
            .iter()
            .find(|entry| entry.kind == "checkpoint")
            .expect("the checkpoint commits beside the boundary");
        let read_back = crate::context::Checkpoint::from_value(&stored.payload)
            .expect("an augmented checkpoint reads back through the strict schema");
        assert_eq!(read_back.provenance.as_deref(), Some("agent+controller"));
        assert_eq!(read_back.files_touched, ["docs/api.md", "src/api.rs"]);
    }

    #[test]
    fn open_work_is_stored_and_reaches_a_restored_session() {
        // Asking for unfinished work and then dropping it would make the
        // prompt field decoration. It is stored on the boundary and carried
        // into the restoration header a cold start reads.
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"work"}))
            .unwrap();
        CompactionController::begin(&db, "s", CompactionReason::BeforeSuspend, 42)
            .unwrap().prompt()
            .unwrap();
        let reply = json!({
            "summary":"halfway through the migration",
            "decisions":[],
            "filesTouched":[],
            "openWork":["backfill the old rows","delete the shim"]
        })
        .to_string();
        CompactionController::handle_output(&db, "s", &reply).unwrap();
        let boundary = store::session_entries(&db, "s")
            .unwrap()
            .into_iter()
            .find(|entry| entry.kind == "compaction")
            .expect("the boundary commits");
        assert_eq!(
            boundary.payload["openWork"],
            json!(["backfill the old rows", "delete the shim"])
        );
    }

    #[test]
    fn a_checkpoint_that_accounts_for_itself_keeps_its_own_provenance() {
        let db = database();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::WorkerResult,
                json!({
                    "status":"completed",
                    "summary":"implemented",
                    "decisions":["Keep the public API"],
                    "filesChanged":["src/api.rs"]
                }),
            )
            .unwrap();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        let complete = json!({
            "summary":"the API stayed put",
            "decisions":["Keep the public API"],
            "filesTouched":["src/api.rs"]
        })
        .to_string();
        CompactionController::handle_output(&db, "s", &complete).unwrap();
        let entries = store::session_entries(&db, "s").unwrap();
        let boundary = entries
            .iter()
            .find(|entry| entry.kind == "compaction")
            .expect("the boundary commits");
        assert_eq!(boundary.payload["provenance"], "agent");
    }

    #[test]
    fn a_fenced_or_prefaced_reply_is_read_rather_than_rejected() {
        // Eight of the checkpoint failures in a month of real use were this,
        // and not one of them had anything wrong with the summary inside.
        for reply in [
            "```json\n{\"summary\":\"fenced\",\"decisions\":[],\"filesTouched\":[]}\n```",
            "Here is the checkpoint:\n{\"summary\":\"fenced\",\"decisions\":[],\"filesTouched\":[]}",
            "{\"summary\":\"fenced\",\"decisions\":[],\"filesTouched\":[]}\n\nLet me know if you need more.",
        ] {
            let db = database();
            SessionForest::new(&db)
                .append("s", EntryKind::AssistantMessage, json!({"text":"work"}))
                .unwrap();
            CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
                .unwrap().prompt()
                .unwrap();
            assert!(
                matches!(
                    CompactionController::handle_output(&db, "s", reply).unwrap(),
                    CheckpointOutcome::Completed { .. }
                ),
                "this reply must commit: {reply}"
            );
        }
    }

    #[test]
    fn a_reply_with_no_object_still_repairs_and_then_fails() {
        // What remains a failure: nothing to read.
        let db = database();
        SessionForest::new(&db)
            .append("s", EntryKind::AssistantMessage, json!({"text":"work"}))
            .unwrap();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 42)
            .unwrap().prompt()
            .unwrap();
        let prose = "I have summarised the session above, let me know what else you need.";
        assert!(matches!(
            CompactionController::handle_output(&db, "s", prose).unwrap(),
            CheckpointOutcome::Repair { .. }
        ));
        assert_eq!(
            CompactionController::handle_output(&db, "s", prose).unwrap(),
            CheckpointOutcome::Failed
        );
        assert!(!store::session_entries(&db, "s")
            .unwrap()
            .iter()
            .any(|entry| entry.kind == "compaction"));
    }

    #[test]
    fn valid_checkpoint_commits_immutable_boundary_and_projects_from_retained_marker() {
        let db = database();
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed","summary":"verified",
            "decisions":["Keep SQLite as source of truth", "Keep SQLite as source of truth"],
            "filesChanged":["src-tauri/src/context.rs", "src-tauri/src/context.rs"]
        })).unwrap();
        CompactionController::begin(&db, "s", CompactionReason::PhaseBoundary, 55)
            .unwrap().prompt()
            .unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        let outcome = CompactionController::handle_output(
            &db,
            "s",
            &valid_output(&pending, "Phase one is complete"),
        )
        .unwrap();
        assert!(matches!(outcome, CheckpointOutcome::Completed { .. }));
        let entries = store::session_entries(&db, "s").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.kind.as_str())
                .collect::<Vec<_>>(),
            vec![
                "user.message",
                "worker.result",
                "compaction.requested",
                "checkpoint",
                "compaction",
                "branch.summary"
            ]
        );
        assert_eq!(entries[4].payload["firstRetainedEntryId"], entries[5].id);
        let branch = SessionForest::new(&db).active_branch("s").unwrap();
        let projection = ContextProjector::project(&branch, 8_000).unwrap();
        assert_eq!(
            projection.restoration_context.unwrap().summary,
            "Phase one is complete"
        );
        assert_eq!(projection.render_entries[0].id, entries[5].id);
        assert_eq!(
            db.query_row(
                "SELECT latest_checkpoint_entry_id FROM session_heads WHERE session_id='s'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            entries[3].id
        );
        let audit: String = db.query_row(
            "SELECT body FROM events WHERE entity_id='s' AND kind='checkpoint.evidence_verified'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&audit).unwrap(),
            json!({"decisionCount":1,"fileCount":1})
        );
    }

    #[test]
    fn completed_compaction_resets_required_evidence_window() {
        let db = database();
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed","summary":"old phase","decisions":["old decision"],"filesChanged":["src/old.rs"]
        })).unwrap();
        CompactionController::begin(&db, "s", CompactionReason::PhaseBoundary, 10)
            .unwrap().prompt()
            .unwrap();
        let first = CompactionController::pending(&db, "s").unwrap().unwrap();
        let first_output = json!({
            "schemaVersion":1,"summary":"first","decisions":["old decision"],"filesTouched":["src/old.rs"],
            "sourceAgent":"s","firstRetainedEntryId":first.first_retained_entry_id,"tokensBefore":first.tokens_before,"reason":first.reason.as_str()
        }).to_string();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", &first_output).unwrap(),
            CheckpointOutcome::Completed { .. }
        ));
        CompactionController::begin(&db, "s", CompactionReason::Manual, 20)
            .unwrap().prompt()
            .unwrap();
        let second = CompactionController::pending(&db, "s").unwrap().unwrap();
        let second_output = json!({
            "schemaVersion":1,"summary":"second","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":second.first_retained_entry_id,"tokensBefore":second.tokens_before,"reason":second.reason.as_str()
        }).to_string();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", &second_output).unwrap(),
            CheckpointOutcome::Completed { .. }
        ));
    }

    #[test]
    fn reconstruction_is_explicitly_labeled_and_auditable() {
        let db = database();
        SessionForest::new(&db)
            .append(
                "s",
                EntryKind::WorkerResult,
                json!({
                    "status":"completed",
                    "summary":"Worker verified the recovery path",
                    "decisions":["Keep raw events"]
                }),
            )
            .unwrap();
        let outcome = CompactionController::reconstruct_from_normalized_events_and_git(
            &db,
            "s",
            " M src-tauri/src/context.rs\n",
        )
        .unwrap();
        assert!(matches!(outcome, CheckpointOutcome::Completed { .. }));
        let compaction = store::session_entries(&db, "s")
            .unwrap()
            .into_iter()
            .find(|entry| entry.kind == "compaction")
            .unwrap();
        assert_eq!(compaction.payload["provenance"], "reconstructed");
        assert_eq!(
            compaction.payload["filesTouched"],
            json!(["src-tauri/src/context.rs"])
        );
    }

    #[test]
    fn long_running_session_survives_three_compactions_with_all_decisions() {
        let db = database();
        for phase in 1..=3 {
            CompactionController::begin(&db, "s", CompactionReason::PhaseBoundary, phase * 100)
                .unwrap().prompt()
                .unwrap();
            let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
            let output = json!({
                "schemaVersion": 1,
                "summary": format!("phase {phase} complete"),
                "decisions": [format!("decision {phase}")],
                "filesTouched": [format!("src/phase-{phase}.rs")],
                "sourceAgent": "s",
                "firstRetainedEntryId": pending.first_retained_entry_id,
                "tokensBefore": pending.tokens_before,
                "reason": pending.reason.as_str(),
            });
            assert!(matches!(
                CompactionController::handle_output(&db, "s", &output.to_string()).unwrap(),
                CheckpointOutcome::Completed { .. }
            ));
            SessionForest::new(&db)
                .append(
                    "s",
                    EntryKind::UserMessage,
                    json!({"text": format!("continue after phase {phase}")}),
                )
                .unwrap();
        }
        let all_entries = store::session_entries(&db, "s").unwrap();
        assert_eq!(
            all_entries
                .iter()
                .filter(|entry| entry.kind == "compaction")
                .count(),
            3
        );
        assert_eq!(all_entries[0].payload["text"], "Keep the durable decision");
        let branch = SessionForest::new(&db).active_branch("s").unwrap();
        let projection = ContextProjector::project(&branch, 8_000).unwrap();
        let restoration = projection.restoration_context.unwrap();
        assert_eq!(restoration.summary, "phase 3 complete");
        assert_eq!(
            restoration.decisions,
            vec!["decision 1", "decision 2", "decision 3"]
        );
        assert!(projection
            .render_entries
            .iter()
            .any(|entry| entry.payload["text"] == "continue after phase 3"));
    }
}
