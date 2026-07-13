//! Pure context projection and checkpoint schema primitives.
//!
//! This module deliberately does not know about SQLite, adapters, or Tauri. Callers
//! supply the already-selected active branch, which keeps projection deterministic
//! and makes compaction policy independently testable.

use crate::model::SessionEntry;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;

pub const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const FOREST_TYPED_SCHEMA_MARKER: &str = "_bridgeTypedSchemaVersion";
const REPOSITORY_STATE_MARKER: &str = "_bridgeRepoState";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checkpoint {
    pub schema_version: u32,
    pub summary: String,
    pub decisions: Vec<String>,
    pub files_touched: Vec<String>,
    pub source_agent: String,
    pub first_retained_entry_id: String,
    pub tokens_before: i64,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

impl Checkpoint {
    /// Parse one exact JSON checkpoint and verify that it belongs to the agent that
    /// was asked to produce it. Markdown fences and trailing prose are rejected.
    pub fn parse_and_validate(
        text: &str,
        expected_source_agent: &str,
    ) -> Result<Self, ContextError> {
        let checkpoint: Self = serde_json::from_str(text)
            .map_err(|error| ContextError::InvalidCheckpoint(error.to_string()))?;
        checkpoint.validate(Some(expected_source_agent))?;
        Ok(checkpoint)
    }

    pub fn validate(&self, expected_source_agent: Option<&str>) -> Result<(), ContextError> {
        if self.schema_version != CHECKPOINT_SCHEMA_VERSION {
            return Err(ContextError::UnsupportedCheckpointVersion(
                self.schema_version,
            ));
        }
        require_non_empty("summary", &self.summary)?;
        require_non_empty("sourceAgent", &self.source_agent)?;
        require_non_empty("firstRetainedEntryId", &self.first_retained_entry_id)?;
        require_non_empty("reason", &self.reason)?;
        if let Some(provenance) = &self.provenance {
            require_non_empty("provenance", provenance)?;
        }
        validate_string_list("decisions", &self.decisions)?;
        validate_string_list("filesTouched", &self.files_touched)?;
        if self.tokens_before < 0 {
            return Err(ContextError::InvalidCheckpoint(
                "tokensBefore must be non-negative".into(),
            ));
        }
        if let Some(expected) = expected_source_agent {
            require_non_empty("expected source agent", expected)?;
            if self.source_agent != expected {
                return Err(ContextError::SourceAgentMismatch {
                    expected: expected.to_owned(),
                    actual: self.source_agent.clone(),
                });
            }
        }
        Ok(())
    }

    fn from_value(value: &Value) -> Result<Self, ContextError> {
        // SessionForest adds this envelope marker after validating producer output.
        // It is storage metadata, not part of the strict checkpoint schema.
        let mut stored = value.clone();
        if let Some(object) = stored.as_object_mut() {
            object.remove(FOREST_TYPED_SCHEMA_MARKER);
            object.remove(REPOSITORY_STATE_MARKER);
        }
        let checkpoint: Self = serde_json::from_value(stored)
            .map_err(|error| ContextError::InvalidCheckpoint(error.to_string()))?;
        checkpoint.validate(None)?;
        Ok(checkpoint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorationContext {
    pub boundary_entry_id: String,
    pub summary: String,
    pub decisions: Vec<String>,
    pub files_touched: Vec<String>,
    pub source_agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextProjection {
    pub render_entries: Vec<SessionEntry>,
    pub restoration_context: Option<RestorationContext>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub token_estimate: i64,
    /// Integer percentage, clamped to 0..=100 for a stable persistence/UI value.
    pub context_pressure: i64,
}

pub struct ContextProjector;

impl ContextProjector {
    /// Project an active branch into provider context.
    ///
    /// Entries are canonicalized by `(sequence, id)`. The newest *valid*
    /// compaction supplies the restoration summary and first retained entry. Raw
    /// worker streams are excluded, while typed `worker.result` entries are reduced
    /// to routing fields plus their canonical evidence IDs.
    pub fn project(
        entries: &[SessionEntry],
        context_window_tokens: i64,
    ) -> Result<ContextProjection, ContextError> {
        if context_window_tokens <= 0 {
            return Err(ContextError::InvalidContextWindow(context_window_tokens));
        }

        let mut branch = entries.to_vec();
        branch.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.id.cmp(&right.id))
        });
        reject_duplicate_entry_ids(&branch)?;

        let model = latest_non_empty(&branch, "model.changed", "model");
        let effort = latest_non_empty(&branch, "effort.changed", "effort");
        let durable_decisions = collect_durable_decisions(&branch);

        let boundary = branch.iter().rev().find_map(|entry| {
            (entry.kind == "compaction")
                .then(|| {
                    Checkpoint::from_value(&entry.payload)
                        .ok()
                        .map(|value| (entry, value))
                })
                .flatten()
        });

        let (retained, restoration_context) = if let Some((boundary_entry, checkpoint)) = boundary {
            let boundary_index = branch
                .iter()
                .position(|entry| entry.id == boundary_entry.id)
                .expect("selected boundary comes from this branch");
            let retained_index = branch
                .iter()
                .position(|entry| entry.id == checkpoint.first_retained_entry_id)
                .ok_or_else(|| ContextError::MissingRetainedEntry {
                    boundary_entry_id: boundary_entry.id.clone(),
                    retained_entry_id: checkpoint.first_retained_entry_id.clone(),
                })?;
            if retained_index <= boundary_index {
                return Err(ContextError::InvalidRetainedBoundary {
                    boundary_entry_id: boundary_entry.id.clone(),
                    retained_entry_id: checkpoint.first_retained_entry_id,
                });
            }
            let mut decisions = durable_decisions;
            append_unique(&mut decisions, checkpoint.decisions.iter().cloned());
            (
                &branch[retained_index..],
                Some(RestorationContext {
                    boundary_entry_id: boundary_entry.id.clone(),
                    summary: checkpoint.summary,
                    decisions,
                    files_touched: checkpoint.files_touched,
                    source_agent: checkpoint.source_agent,
                    provenance: checkpoint.provenance,
                }),
            )
        } else {
            (&branch[..], None)
        };

        let render_entries = retained
            .iter()
            .filter_map(|entry| project_render_entry(entry))
            .collect::<Vec<_>>();
        let restoration_tokens = restoration_context
            .as_ref()
            .map(estimate_serialized_tokens)
            .unwrap_or_default();
        let token_estimate = render_entries
            .iter()
            .map(estimate_entry_tokens)
            .fold(0_i64, i64::saturating_add)
            .saturating_add(restoration_tokens);
        let context_pressure = token_estimate
            .saturating_mul(100)
            .checked_div(context_window_tokens)
            .unwrap_or(100)
            .clamp(0, 100);

        Ok(ContextProjection {
            render_entries,
            restoration_context,
            model,
            effort,
            token_estimate,
            context_pressure,
        })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContextError {
    #[error("invalid checkpoint: {0}")]
    InvalidCheckpoint(String),
    #[error("unsupported checkpoint schema version {0}")]
    UnsupportedCheckpointVersion(u32),
    #[error("checkpoint source agent mismatch: expected {expected}, got {actual}")]
    SourceAgentMismatch { expected: String, actual: String },
    #[error("context window must be positive, got {0}")]
    InvalidContextWindow(i64),
    #[error("duplicate active-branch entry id: {0}")]
    DuplicateEntryId(String),
    #[error("compaction {boundary_entry_id} retains missing entry {retained_entry_id}")]
    MissingRetainedEntry {
        boundary_entry_id: String,
        retained_entry_id: String,
    },
    #[error(
        "compaction {boundary_entry_id} must retain an entry after itself, got {retained_entry_id}"
    )]
    InvalidRetainedBoundary {
        boundary_entry_id: String,
        retained_entry_id: String,
    },
}

fn require_non_empty(field: &str, value: &str) -> Result<(), ContextError> {
    if value.trim().is_empty() {
        Err(ContextError::InvalidCheckpoint(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_string_list(field: &str, values: &[String]) -> Result<(), ContextError> {
    let mut seen = HashSet::new();
    for value in values {
        require_non_empty(&format!("{field} item"), value)?;
        if !seen.insert(value) {
            return Err(ContextError::InvalidCheckpoint(format!(
                "{field} contains a duplicate item"
            )));
        }
    }
    Ok(())
}

fn reject_duplicate_entry_ids(entries: &[SessionEntry]) -> Result<(), ContextError> {
    let mut seen = HashSet::new();
    for entry in entries {
        if !seen.insert(entry.id.as_str()) {
            return Err(ContextError::DuplicateEntryId(entry.id.clone()));
        }
    }
    Ok(())
}

fn latest_non_empty(entries: &[SessionEntry], kind: &str, field: &str) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        (entry.kind == kind)
            .then(|| entry.payload.get(field).and_then(Value::as_str))
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn collect_durable_decisions(entries: &[SessionEntry]) -> Vec<String> {
    let mut decisions = Vec::new();
    for entry in entries {
        match entry.kind.as_str() {
            "checkpoint" | "compaction" => {
                if let Ok(checkpoint) = Checkpoint::from_value(&entry.payload) {
                    append_unique(&mut decisions, checkpoint.decisions);
                }
            }
            "worker.result" => {
                let values = entry
                    .payload
                    .get("decisions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                append_unique(&mut decisions, values);
            }
            _ => {}
        }
    }
    decisions
}

fn append_unique(target: &mut Vec<String>, values: impl IntoIterator<Item = String>) {
    let mut seen = target.iter().cloned().collect::<HashSet<_>>();
    for value in values {
        if seen.insert(value.clone()) {
            target.push(value);
        }
    }
}

fn project_render_entry(entry: &SessionEntry) -> Option<SessionEntry> {
    if matches!(
        entry.kind.as_str(),
        "checkpoint" | "compaction" | "compaction.requested" | "compaction.failed"
    ) {
        return None;
    }
    if entry.kind != "worker.result" && entry.context_visibility != "eligible" {
        return None;
    }
    if entry.kind != "worker.result" {
        let mut projected = entry.clone();
        if let Some(object) = projected.payload.as_object_mut() {
            object.remove(REPOSITORY_STATE_MARKER);
        }
        return Some(projected);
    }

    let allowed = [
        "status",
        "summary",
        "suggestedNextAction",
        "suggestedRole",
        "suggestedTask",
        "childSessionId",
    ];
    let mut payload = Map::new();
    for field in allowed {
        if let Some(value) = entry.payload.get(field) {
            payload.insert(field.to_owned(), value.clone());
        }
    }
    let summary = payload
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    payload.insert("summary".into(), Value::String(summary.to_owned()));
    payload.insert("evidenceId".into(), Value::String(entry.id.clone()));
    let mut projected = entry.clone();
    projected.payload = Value::Object(payload);
    projected.token_estimate = None;
    Some(projected)
}

fn estimate_entry_tokens(entry: &SessionEntry) -> i64 {
    entry
        .token_estimate
        .filter(|value| *value >= 0)
        .unwrap_or_else(|| {
            let payload_bytes = serde_json::to_vec(&entry.payload)
                .map(|value| value.len())
                .unwrap_or_default();
            estimate_bytes(entry.kind.len().saturating_add(payload_bytes))
        })
}

fn estimate_serialized_tokens(value: &impl Serialize) -> i64 {
    serde_json::to_vec(value)
        .map(|value| estimate_bytes(value.len()))
        .unwrap_or_default()
}

fn estimate_bytes(bytes: usize) -> i64 {
    i64::try_from(bytes.saturating_add(3) / 4).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(sequence: i64, kind: &str, payload: Value) -> SessionEntry {
        SessionEntry {
            id: format!("entry-{sequence}"),
            session_id: "session".into(),
            parent_entry_id: (sequence > 1).then(|| format!("entry-{}", sequence - 1)),
            sequence,
            kind: kind.into(),
            payload,
            provider_event_id: None,
            context_visibility: "eligible".into(),
            token_estimate: None,
            created_at: format!("2026-01-01T00:00:{sequence:02}Z"),
        }
    }

    fn checkpoint(summary: &str, retained: &str, decisions: &[&str], source: &str) -> Value {
        json!({
            "schemaVersion": CHECKPOINT_SCHEMA_VERSION,
            "summary": summary,
            "decisions": decisions,
            "filesTouched": ["src/lib.rs"],
            "sourceAgent": source,
            "firstRetainedEntryId": retained,
            "tokensBefore": 1200,
            "reason": "pressure"
        })
    }

    #[test]
    fn checkpoint_schema_is_strict_versioned_and_owned() {
        let value = checkpoint("state", "entry-9", &["keep decision"], "agent-a");
        let parsed = Checkpoint::parse_and_validate(&value.to_string(), "agent-a").unwrap();
        assert_eq!(parsed.source_agent, "agent-a");
        assert!(matches!(
            Checkpoint::parse_and_validate(&value.to_string(), "agent-b"),
            Err(ContextError::SourceAgentMismatch { .. })
        ));

        let mut unsupported = value.clone();
        unsupported["schemaVersion"] = json!(2);
        assert_eq!(
            Checkpoint::parse_and_validate(&unsupported.to_string(), "agent-a"),
            Err(ContextError::UnsupportedCheckpointVersion(2))
        );
        let mut unknown = value.clone();
        unknown["rawTranscript"] = json!("must never be accepted");
        assert!(matches!(
            Checkpoint::parse_and_validate(&unknown.to_string(), "agent-a"),
            Err(ContextError::InvalidCheckpoint(_))
        ));
        assert!(
            Checkpoint::parse_and_validate(&format!("```json\n{}\n```", value), "agent-a").is_err()
        );
    }

    #[test]
    fn checkpoint_rejects_missing_malformed_and_duplicate_fields() {
        let mut missing = checkpoint("state", "entry-9", &[], "agent-a");
        missing.as_object_mut().unwrap().remove("filesTouched");
        assert!(Checkpoint::parse_and_validate(&missing.to_string(), "agent-a").is_err());

        let mut negative = checkpoint("state", "entry-9", &[], "agent-a");
        negative["tokensBefore"] = json!(-1);
        assert!(Checkpoint::parse_and_validate(&negative.to_string(), "agent-a").is_err());

        let duplicate = checkpoint("state", "entry-9", &["same", "same"], "agent-a");
        assert!(Checkpoint::parse_and_validate(&duplicate.to_string(), "agent-a").is_err());

        let mut reconstructed = checkpoint("state", "entry-9", &[], "agent-a");
        reconstructed["provenance"] = json!("reconstructed");
        assert_eq!(
            Checkpoint::parse_and_validate(&reconstructed.to_string(), "agent-a")
                .unwrap()
                .provenance
                .as_deref(),
            Some("reconstructed")
        );
    }

    #[test]
    fn projection_is_deterministic_and_newest_of_three_boundaries_wins() {
        let mut third = checkpoint("third", "entry-8", &["decision three"], "orchestrator");
        third[FOREST_TYPED_SCHEMA_MARKER] = json!(1);
        let branch = vec![
            entry(1, "user.message", json!({"text":"initial"})),
            entry(2, "assistant.message", json!({"text":"old answer"})),
            entry(
                3,
                "compaction",
                checkpoint("first", "entry-4", &["decision one"], "orchestrator"),
            ),
            entry(4, "user.message", json!({"text":"second phase"})),
            entry(
                5,
                "compaction",
                checkpoint("second", "entry-6", &["decision two"], "orchestrator"),
            ),
            entry(6, "assistant.message", json!({"text":"third phase"})),
            entry(7, "compaction", third),
            entry(8, "user.message", json!({"text":"retained"})),
            entry(9, "assistant.message", json!({"text":"latest"})),
        ];
        let first = ContextProjector::project(&branch, 8_000).unwrap();
        let second = ContextProjector::project(&branch, 8_000).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first
                .render_entries
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            vec!["entry-8", "entry-9"]
        );
        let restoration = first.restoration_context.unwrap();
        assert_eq!(restoration.summary, "third");
        assert_eq!(restoration.boundary_entry_id, "entry-7");
        assert_eq!(
            restoration.decisions,
            vec!["decision one", "decision two", "decision three"]
        );
    }

    #[test]
    fn projection_extracts_latest_model_and_effort_across_boundary() {
        let branch = vec![
            entry(1, "model.changed", json!({"model":"old"})),
            entry(2, "effort.changed", json!({"effort":"medium"})),
            entry(
                3,
                "compaction",
                checkpoint("state", "entry-4", &[], "agent"),
            ),
            entry(4, "user.message", json!({"text":"continue"})),
            entry(5, "model.changed", json!({"model":"new"})),
            entry(6, "effort.changed", json!({"effort":"high"})),
        ];
        let projection = ContextProjector::project(&branch, 8_000).unwrap();
        assert_eq!(projection.model.as_deref(), Some("new"));
        assert_eq!(projection.effort.as_deref(), Some("high"));
    }

    #[test]
    fn projection_never_exposes_repository_storage_metadata() {
        let branch = vec![entry(
            1,
            "user.message",
            json!({
                "text": "continue",
                REPOSITORY_STATE_MARKER: {"status":"dirty","head":"abc","dirtyHash":"123"}
            }),
        )];
        let projection = ContextProjector::project(&branch, 8_000).unwrap();
        assert_eq!(projection.render_entries[0].payload["text"], "continue");
        assert!(projection.render_entries[0]
            .payload
            .get(REPOSITORY_STATE_MARKER)
            .is_none());
    }

    #[test]
    fn token_estimate_is_stable_for_split_and_large_turns() {
        let mut first = entry(1, "user.message", json!({"text":"a".repeat(8_001)}));
        first.token_estimate = Some(2_100);
        let second = entry(2, "assistant.message", json!({"text":"b".repeat(7_999)}));
        let branch = vec![first, second];
        let projection = ContextProjector::project(&branch, 4_000).unwrap();
        assert_eq!(
            projection.token_estimate,
            ContextProjector::project(&branch, 4_000)
                .unwrap()
                .token_estimate
        );
        assert!(projection.token_estimate > 4_000);
        assert_eq!(projection.context_pressure, 100);
    }

    #[test]
    fn orchestrator_gets_typed_worker_summary_but_never_raw_logs() {
        let mut raw = entry(
            1,
            "assistant.message",
            json!({"text":"secret raw worker transcript"}),
        );
        raw.context_visibility = "worker_raw".into();
        let result = entry(
            2,
            "worker.result",
            json!({
                "schemaVersion": 1,
                "status": "completed",
                "summary": "Tests pass",
                "decisions": ["keep sqlite"],
                "rawTranscript": "secret raw worker transcript",
                "providerLog": {"tokens": 10}
            }),
        );
        let projection = ContextProjector::project(&[raw, result], 8_000).unwrap();
        assert_eq!(projection.render_entries.len(), 1);
        assert_eq!(projection.render_entries[0].kind, "worker.result");
        assert_eq!(
            projection.render_entries[0].payload["summary"],
            "Tests pass"
        );
        assert_eq!(
            projection.render_entries[0].payload["evidenceId"],
            "entry-2"
        );
        assert!(projection.render_entries[0]
            .payload
            .get("decisions")
            .is_none());
        assert!(projection.render_entries[0]
            .payload
            .get("rawTranscript")
            .is_none());
        assert!(projection.render_entries[0]
            .payload
            .get("providerLog")
            .is_none());
        assert!(!serde_json::to_string(&projection)
            .unwrap()
            .contains("secret raw"));
    }

    #[test]
    fn invalid_latest_compaction_falls_back_to_newest_valid_boundary() {
        let mut malformed = checkpoint("bad", "entry-6", &[], "agent");
        malformed["schemaVersion"] = json!(99);
        let branch = vec![
            entry(1, "user.message", json!({"text":"old"})),
            entry(
                2,
                "compaction",
                checkpoint("valid", "entry-3", &[], "agent"),
            ),
            entry(3, "user.message", json!({"text":"retained"})),
            entry(4, "compaction", malformed),
            entry(5, "assistant.message", json!({"text":"latest"})),
        ];
        let projection = ContextProjector::project(&branch, 8_000).unwrap();
        assert_eq!(projection.restoration_context.unwrap().summary, "valid");
        assert_eq!(projection.render_entries[0].id, "entry-3");
    }

    #[test]
    fn missing_retained_entry_and_invalid_window_fail_explicitly() {
        let branch = vec![entry(
            1,
            "compaction",
            checkpoint("state", "missing", &[], "agent"),
        )];
        assert!(matches!(
            ContextProjector::project(&branch, 8_000),
            Err(ContextError::MissingRetainedEntry { .. })
        ));
        assert_eq!(
            ContextProjector::project(&[], 0),
            Err(ContextError::InvalidContextWindow(0))
        );

        let backwards = vec![
            entry(1, "user.message", json!({"text":"old"})),
            entry(
                2,
                "compaction",
                checkpoint("state", "entry-1", &[], "agent"),
            ),
        ];
        assert!(matches!(
            ContextProjector::project(&backwards, 8_000),
            Err(ContextError::InvalidRetainedBoundary { .. })
        ));
    }
}
