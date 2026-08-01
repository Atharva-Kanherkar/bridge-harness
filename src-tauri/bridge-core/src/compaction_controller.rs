use crate::{
    context::Checkpoint,
    model::{SessionEntry, SEMANTIC_EVENT_SCHEMA_VERSION},
    session_forest::{append_in_transaction, EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::Utc;
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
    fires.then_some(state.reason).ok_or(SuppressionReason::BelowThreshold)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCompaction {
    pub reason: CompactionReason,
    pub attempt: u8,
    pub tokens_before: i64,
    pub requested_at: String,
    pub first_retained_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckpointEvidence {
    decisions: BTreeSet<String>,
    files_touched: BTreeSet<String>,
}

impl CheckpointEvidence {
    fn from_active_history(db: &Connection, session_id: &str) -> Result<Self, BridgeError> {
        let branch = SessionForest::new(db)
            .active_branch(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let start = branch.iter().rposition(|entry| entry.kind == "compaction")
            .map(|index| index + 1).unwrap_or(0);
        let mut evidence = Self { decisions: BTreeSet::new(), files_touched: BTreeSet::new() };
        for entry in &branch[start..] {
            if matches!(entry.kind.as_str(), "worker.result" | "checkpoint") {
                collect_strings(&entry.payload, "decisions", &mut evidence.decisions);
            }
            if entry.kind == "worker.result" {
                collect_strings(&entry.payload, "filesChanged", &mut evidence.files_touched);
            } else if entry.kind == "artifact.created" {
                if let Some(path) = entry.payload.get("path").and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()) {
                    evidence.files_touched.insert(path.to_owned());
                }
            }
        }
        Ok(evidence)
    }

    fn verify(&self, checkpoint: &Checkpoint) -> Result<(), String> {
        let actual_decisions = checkpoint.decisions.iter().map(|value| value.trim().to_owned()).collect::<BTreeSet<_>>();
        let actual_files = checkpoint.files_touched.iter().map(|value| value.trim().to_owned()).collect::<BTreeSet<_>>();
        let missing_decisions = self.decisions.difference(&actual_decisions).cloned().collect::<Vec<_>>();
        let missing_files = self.files_touched.difference(&actual_files).cloned().collect::<Vec<_>>();
        if missing_decisions.is_empty() && missing_files.is_empty() {
            return Ok(());
        }
        Err(format!(
            "checkpoint omits durable evidence; missing decisions: {}; missing files: {}",
            if missing_decisions.is_empty() { "none".into() } else { missing_decisions.join(" | ") },
            if missing_files.is_empty() { "none".into() } else { missing_files.join(" | ") },
        ))
    }
}

fn collect_strings(payload: &Value, field: &str, target: &mut BTreeSet<String>) {
    for value in payload.get(field).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let value = value.trim();
        if !value.is_empty() {
            target.insert(value.to_owned());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointOutcome {
    Completed { checkpoint_entry_id: String, compaction_entry_id: String },
    Repair { prompt: String },
    Failed,
    NotPending,
}

pub struct CompactionController;

impl CompactionController {
    pub fn checkpoint_prompt(
        session_id: &str,
        pending: &PendingCompaction,
        repair_error: Option<&str>,
    ) -> String {
        let repair = repair_error
            .map(|error| format!(" Your previous response was invalid: {error}."))
            .unwrap_or_default();
        format!(
            "Produce a checkpoint matching this exact JSON schema; do no more work. Return JSON only: {{\"schemaVersion\":1,\"summary\":\"non-empty\",\"decisions\":[\"durable decision\"],\"filesTouched\":[\"relative/path\"],\"sourceAgent\":\"{session_id}\",\"firstRetainedEntryId\":\"{}\",\"tokensBefore\":{},\"reason\":\"{}\"}}.{repair}",
            pending.first_retained_entry_id,
            pending.tokens_before,
            pending.reason.as_str(),
        )
    }

    pub fn begin(
        db: &Connection,
        session_id: &str,
        reason: CompactionReason,
        tokens_before: i64,
    ) -> Result<Option<String>, BridgeError> {
        if Self::pending(db, session_id)?.is_some() {
            return Ok(None);
        }
        let first_retained_entry_id = Uuid::new_v4().to_string();
        let pending = PendingCompaction {
            reason,
            attempt: 0,
            tokens_before: tokens_before.max(0),
            requested_at: Utc::now().to_rfc3339(),
            first_retained_entry_id,
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
                }),
            )
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        Ok(Some(Self::checkpoint_prompt(session_id, &pending, None)))
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
        let checkpoint = match Checkpoint::parse_and_validate(output, session_id) {
            Ok(checkpoint) => checkpoint,
            Err(error) if pending.attempt == 0 => {
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
                            "repairOf": error.to_string(),
                        }),
                    )
                    .map_err(|forest_error| BridgeError::Invalid(forest_error.to_string()))?;
                return Ok(CheckpointOutcome::Repair {
                    prompt: Self::checkpoint_prompt(session_id, &PendingCompaction {
                        attempt: 1,
                        ..pending
                    }, Some(&error.to_string())),
                });
            }
            Err(error) => {
                Self::record_failure(db, session_id, &error.to_string(), pending.attempt)?;
                return Ok(CheckpointOutcome::Failed);
            }
        };
        if checkpoint.first_retained_entry_id != pending.first_retained_entry_id
            || checkpoint.tokens_before != pending.tokens_before
            || checkpoint.reason != pending.reason.as_str()
        {
            let error = "checkpoint metadata does not match its controller request";
            if pending.attempt == 0 {
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
                            "repairOf": error,
                        }),
                    )
                    .map_err(|forest_error| BridgeError::Invalid(forest_error.to_string()))?;
                return Ok(CheckpointOutcome::Repair {
                    prompt: Self::checkpoint_prompt(
                        session_id,
                        &PendingCompaction {
                            attempt: 1,
                            ..pending
                        },
                        Some(error),
                    ),
                });
            }
            Self::record_failure(db, session_id, error, pending.attempt)?;
            return Ok(CheckpointOutcome::Failed);
        }
        let evidence = CheckpointEvidence::from_active_history(db, session_id)?;
        if let Err(error) = evidence.verify(&checkpoint) {
            return Self::reject_incomplete_checkpoint(db, session_id, &pending, &error);
        }
        Self::record_checkpoint(db, session_id, checkpoint, pending, "agent", Some(&evidence))
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
        };
        Self::record_checkpoint(db, session_id, checkpoint, pending, "reconstructed", None)
    }

    pub fn reconstruct_from_normalized_events_and_git(
        db: &Connection,
        session_id: &str,
        git_status: &str,
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
            CompactionReason::PhaseBoundary,
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
        SessionForest::new(db)
            .append(
                session_id,
                EntryKind::CompactionFailed,
                json!({"reason": reason, "attempt": attempt, "failedAt": Utc::now().to_rfc3339()}),
            )
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        store::event(db, "compaction", "compaction.failed", session_id, reason)?;
        Ok(())
    }

    fn reject_incomplete_checkpoint(
        db: &Connection,
        session_id: &str,
        pending: &PendingCompaction,
        error: &str,
    ) -> Result<CheckpointOutcome, BridgeError> {
        if pending.attempt == 0 {
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
                        "repairOf": error,
                    }),
                )
                .map_err(|forest_error| BridgeError::Invalid(forest_error.to_string()))?;
            return Ok(CheckpointOutcome::Repair {
                prompt: Self::checkpoint_prompt(
                    session_id,
                    &PendingCompaction { attempt: 1, ..pending.clone() },
                    Some(error),
                ),
            });
        }
        Self::record_failure(db, session_id, error, pending.attempt)?;
        Ok(CheckpointOutcome::Failed)
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
            "compaction" | "compaction.failed" => return Ok(None),
            "compaction.requested" => {
                let reason = entry
                    .payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .and_then(CompactionReason::parse)
                    .ok_or_else(|| BridgeError::Invalid("invalid compaction reason".into()))?;
                return Ok(Some(PendingCompaction {
                    reason,
                    attempt: entry.payload.get("attempt").and_then(Value::as_u64).unwrap_or(0)
                        as u8,
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
                }));
            }
            _ => {}
        }
    }
    Ok(None)
}

pub fn active_token_estimate(db: &Connection, session_id: &str) -> Result<i64, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(branch
        .iter()
        .map(|entry| {
            entry.token_estimate.unwrap_or_else(|| {
                let bytes = entry.payload.to_string().len() as i64;
                (bytes + 3) / 4
            })
        })
        .sum())
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
            .unwrap()
            .unwrap();
        assert!(matches!(
            CompactionController::handle_output(&db, "s", "not json").unwrap(),
            CheckpointOutcome::Repair { .. }
        ));
        assert_eq!(CompactionController::pending(&db, "s").unwrap().unwrap().attempt, 1);
        assert_eq!(
            CompactionController::handle_output(&db, "s", "still invalid").unwrap(),
            CheckpointOutcome::Failed
        );
        let entries = store::session_entries(&db, "s").unwrap();
        assert_eq!(entries[0].kind, "user.message");
        assert_eq!(entries[0].payload["text"], "Keep the durable decision");
        assert_eq!(entries.last().unwrap().kind, "compaction.failed");
        assert!(CompactionController::pending(&db, "s").unwrap().is_none());
    }

    #[test]
    fn schema_valid_checkpoint_with_missing_durable_evidence_repairs_then_fails() {
        let db = database();
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed",
            "summary":"implemented",
            "decisions":["Keep the public API", "Keep the public API"],
            "filesChanged":["src/api.rs", "src/api.rs"]
        })).unwrap();
        SessionForest::new(&db).append(
            "s",
            EntryKind::ArtifactCreated,
            json!({"path":"docs/api.md"}),
        ).unwrap();
        CompactionController::begin(&db, "s", CompactionReason::Manual, 42).unwrap().unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        let incomplete = json!({
            "schemaVersion":1,"summary":"looks complete","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":pending.first_retained_entry_id,
            "tokensBefore":pending.tokens_before,"reason":pending.reason.as_str()
        }).to_string();
        let CheckpointOutcome::Repair { prompt } = CompactionController::handle_output(&db, "s", &incomplete).unwrap() else { panic!("expected repair") };
        assert!(prompt.contains("Keep the public API"));
        assert!(prompt.contains("src/api.rs"));
        assert!(prompt.contains("docs/api.md"));
        assert_eq!(CompactionController::handle_output(&db, "s", &incomplete).unwrap(), CheckpointOutcome::Failed);
        assert!(!store::session_entries(&db, "s").unwrap().iter().any(|entry| entry.kind == "compaction"));
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
            .unwrap()
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
            entries.iter().map(|entry| entry.kind.as_str()).collect::<Vec<_>>(),
            vec!["user.message", "worker.result", "compaction.requested", "checkpoint", "compaction", "branch.summary"]
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
        assert_eq!(serde_json::from_str::<Value>(&audit).unwrap(), json!({"decisionCount":1,"fileCount":1}));
    }

    #[test]
    fn completed_compaction_resets_required_evidence_window() {
        let db = database();
        SessionForest::new(&db).append("s", EntryKind::WorkerResult, json!({
            "status":"completed","summary":"old phase","decisions":["old decision"],"filesChanged":["src/old.rs"]
        })).unwrap();
        CompactionController::begin(&db, "s", CompactionReason::PhaseBoundary, 10).unwrap().unwrap();
        let first = CompactionController::pending(&db, "s").unwrap().unwrap();
        let first_output = json!({
            "schemaVersion":1,"summary":"first","decisions":["old decision"],"filesTouched":["src/old.rs"],
            "sourceAgent":"s","firstRetainedEntryId":first.first_retained_entry_id,"tokensBefore":first.tokens_before,"reason":first.reason.as_str()
        }).to_string();
        assert!(matches!(CompactionController::handle_output(&db, "s", &first_output).unwrap(), CheckpointOutcome::Completed { .. }));
        CompactionController::begin(&db, "s", CompactionReason::Manual, 20).unwrap().unwrap();
        let second = CompactionController::pending(&db, "s").unwrap().unwrap();
        let second_output = json!({
            "schemaVersion":1,"summary":"second","decisions":[],"filesTouched":[],
            "sourceAgent":"s","firstRetainedEntryId":second.first_retained_entry_id,"tokensBefore":second.tokens_before,"reason":second.reason.as_str()
        }).to_string();
        assert!(matches!(CompactionController::handle_output(&db, "s", &second_output).unwrap(), CheckpointOutcome::Completed { .. }));
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
                .unwrap()
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
            all_entries.iter().filter(|entry| entry.kind == "compaction").count(),
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
