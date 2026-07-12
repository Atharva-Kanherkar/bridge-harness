use crate::{model::SessionEntry, store, BridgeError};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    UserMessage,
    AssistantMessage,
    ToolStarted,
    ToolCompleted,
    ApprovalRequested,
    ApprovalResolved,
    DelegationRequested,
    DelegationApproved,
    DelegationRejected,
    WorkerResult,
    Checkpoint,
    CompactionRequested,
    Compaction,
    CompactionFailed,
    BranchSummary,
    ModelChanged,
    EffortChanged,
    SessionStatus,
    SessionResumeFailed,
    ArtifactCreated,
    Legacy(String),
}

impl EntryKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UserMessage => "user.message",
            Self::AssistantMessage => "assistant.message",
            Self::ToolStarted => "tool.started",
            Self::ToolCompleted => "tool.completed",
            Self::ApprovalRequested => "approval.requested",
            Self::ApprovalResolved => "approval.resolved",
            Self::DelegationRequested => "delegation.requested",
            Self::DelegationApproved => "delegation.approved",
            Self::DelegationRejected => "delegation.rejected",
            Self::WorkerResult => "worker.result",
            Self::Checkpoint => "checkpoint",
            Self::CompactionRequested => "compaction.requested",
            Self::Compaction => "compaction",
            Self::CompactionFailed => "compaction.failed",
            Self::BranchSummary => "branch.summary",
            Self::ModelChanged => "model.changed",
            Self::EffortChanged => "effort.changed",
            Self::SessionStatus => "session.status",
            Self::SessionResumeFailed => "session.resume_failed",
            Self::ArtifactCreated => "artifact.created",
            Self::Legacy(value) => value,
        }
    }

    pub fn from_storage(value: &str) -> Self {
        match value {
            "user.message" => Self::UserMessage,
            "assistant.message" => Self::AssistantMessage,
            "tool.started" => Self::ToolStarted,
            "tool.completed" => Self::ToolCompleted,
            "approval.requested" => Self::ApprovalRequested,
            "approval.resolved" => Self::ApprovalResolved,
            "delegation.requested" => Self::DelegationRequested,
            "delegation.approved" => Self::DelegationApproved,
            "delegation.rejected" => Self::DelegationRejected,
            "worker.result" => Self::WorkerResult,
            "checkpoint" => Self::Checkpoint,
            "compaction.requested" => Self::CompactionRequested,
            "compaction" => Self::Compaction,
            "compaction.failed" => Self::CompactionFailed,
            "branch.summary" => Self::BranchSummary,
            "model.changed" => Self::ModelChanged,
            "effort.changed" => Self::EffortChanged,
            "session.status" => Self::SessionStatus,
            "session.resume_failed" => Self::SessionResumeFailed,
            "artifact.created" => Self::ArtifactCreated,
            other => Self::Legacy(other.to_owned()),
        }
    }

    fn ensure_appendable(&self) -> Result<(), ForestError> {
        match self {
            Self::Legacy(kind) => Err(ForestError::UnsupportedEntryKind(kind.clone())),
            _ => Ok(()),
        }
    }

    pub fn validate_payload(&self, payload: &Value) -> Result<(), ForestError> {
        self.ensure_appendable()?;
        if !payload.is_object() {
            return Err(ForestError::InvalidPayload {
                kind: self.as_str().to_owned(),
                reason: "payload must be a JSON object".into(),
            });
        }
        match self {
            Self::UserMessage | Self::AssistantMessage => {
                require_string(payload, self, &["text"])?;
            }
            Self::ToolStarted | Self::ToolCompleted => {
                require_one_string(payload, self, &["toolId", "itemId"])?;
            }
            Self::ApprovalRequested | Self::ApprovalResolved => {
                require_one_string(payload, self, &["approvalId", "itemId"])?;
            }
            Self::DelegationRequested | Self::DelegationApproved | Self::DelegationRejected => {
                require_one_string(payload, self, &["requestId", "itemId"])?;
            }
            Self::WorkerResult => {
                require_string(payload, self, &["status"])?;
            }
            Self::Checkpoint => {
                require_number(payload, self, "schemaVersion")?;
                require_string(payload, self, &["summary"])?;
            }
            Self::CompactionRequested => {
                require_string(payload, self, &["reason"])?;
            }
            Self::Compaction => {
                require_number(payload, self, "schemaVersion")?;
                require_string(payload, self, &["summary"])?;
            }
            Self::CompactionFailed | Self::SessionResumeFailed => {
                require_string(payload, self, &["reason"])?;
            }
            Self::BranchSummary => {
                require_string(payload, self, &["summary"])?;
            }
            Self::ModelChanged => {
                require_string(payload, self, &["model"])?;
            }
            Self::EffortChanged => {
                require_string(payload, self, &["effort"])?;
            }
            Self::SessionStatus => {
                require_string(payload, self, &["status"])?;
                if matches!(
                    payload.get("status").and_then(Value::as_str),
                    Some("stopped" | "cancelled")
                ) {
                    require_string(payload, self, &["reason"])?;
                }
            }
            Self::ArtifactCreated => {
                require_string(payload, self, &["path"])?;
            }
            Self::Legacy(_) => unreachable!("legacy kinds are rejected before validation"),
        }
        Ok(())
    }
}

fn require_string(payload: &Value, kind: &EntryKind, fields: &[&str]) -> Result<(), ForestError> {
    if fields.iter().all(|field| {
        payload
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return Ok(());
    }
    Err(ForestError::InvalidPayload {
        kind: kind.as_str().to_owned(),
        reason: format!("required non-empty string field(s): {}", fields.join(", ")),
    })
}

fn require_one_string(
    payload: &Value,
    kind: &EntryKind,
    alternatives: &[&str],
) -> Result<(), ForestError> {
    if alternatives.iter().any(|field| {
        payload
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return Ok(());
    }
    Err(ForestError::InvalidPayload {
        kind: kind.as_str().to_owned(),
        reason: format!(
            "one non-empty string field is required: {}",
            alternatives.join(" or ")
        ),
    })
}

fn require_number(payload: &Value, kind: &EntryKind, field: &str) -> Result<(), ForestError> {
    if payload.get(field).is_some_and(Value::is_number) {
        return Ok(());
    }
    Err(ForestError::InvalidPayload {
        kind: kind.as_str().to_owned(),
        reason: format!("required numeric field: {field}"),
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ForestError {
    #[error("database: {0}")]
    Database(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("entry not found: {entry_id} in session {session_id}")]
    EntryNotFound {
        session_id: String,
        entry_id: String,
    },
    #[error("unsupported entry kind for append: {0}")]
    UnsupportedEntryKind(String),
    #[error("invalid payload for {kind}: {reason}")]
    InvalidPayload { kind: String, reason: String },
    #[error("entry {entry_id} references missing parent {parent_entry_id}")]
    OrphanParent {
        entry_id: String,
        parent_entry_id: String,
    },
    #[error("entry {entry_id} in session {session_id} references parent {parent_entry_id} in session {parent_session_id}")]
    CrossSessionParent {
        session_id: String,
        entry_id: String,
        parent_entry_id: String,
        parent_session_id: String,
    },
    #[error("cycle detected at entry {entry_id} in session {session_id}")]
    Cycle {
        session_id: String,
        entry_id: String,
    },
    #[error("session {session_id} has invalid active head {entry_id}")]
    InvalidHead {
        session_id: String,
        entry_id: String,
    },
}

impl From<rusqlite::Error> for ForestError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

impl From<BridgeError> for ForestError {
    fn from(value: BridgeError) -> Self {
        match value {
            BridgeError::Db(error) => Self::Database(error.to_string()),
            other => Self::Storage(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedSession {
    pub session_id: String,
    pub findings: Vec<ForestError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    pub healthy_sessions: Vec<String>,
    pub quarantined_sessions: Vec<QuarantinedSession>,
}

pub struct SessionForest<'connection> {
    db: &'connection Connection,
}

impl<'connection> SessionForest<'connection> {
    pub fn new(db: &'connection Connection) -> Self {
        Self { db }
    }

    pub fn append(
        &self,
        session_id: &str,
        kind: EntryKind,
        payload: Value,
    ) -> Result<SessionEntry, ForestError> {
        let parent = self.active_head_id(session_id)?;
        self.append_to(session_id, parent.as_deref(), kind, payload)
    }

    pub fn append_to(
        &self,
        session_id: &str,
        parent_entry_id: Option<&str>,
        kind: EntryKind,
        payload: Value,
    ) -> Result<SessionEntry, ForestError> {
        kind.validate_payload(&payload)?;
        self.ensure_session(session_id)?;
        if let Some(parent_entry_id) = parent_entry_id {
            self.branch_to_leaf(session_id, parent_entry_id)?;
        }
        let transaction = self.db.unchecked_transaction()?;
        let entry = store::append_session_entry_tx(
            &transaction,
            session_id,
            parent_entry_id,
            kind.as_str(),
            &payload,
            None,
            "eligible",
            None,
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    pub fn append_branch_summary(
        &self,
        session_id: &str,
        parent_entry_id: Option<&str>,
        summary: &str,
    ) -> Result<SessionEntry, ForestError> {
        self.append_to(
            session_id,
            parent_entry_id,
            EntryKind::BranchSummary,
            serde_json::json!({ "summary": summary }),
        )
    }

    pub fn move_head(&self, session_id: &str, entry_id: Option<&str>) -> Result<(), ForestError> {
        self.ensure_session(session_id)?;
        if let Some(entry_id) = entry_id {
            self.ensure_entry_in_session(session_id, entry_id)?;
        }
        let now = chrono::Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,updated_at)
             VALUES(?1,?2,'fresh',?3)
             ON CONFLICT(session_id) DO UPDATE SET active_entry_id=excluded.active_entry_id,updated_at=excluded.updated_at",
            params![session_id, entry_id, now],
        )?;
        Ok(())
    }

    pub fn active_branch(&self, session_id: &str) -> Result<Vec<SessionEntry>, ForestError> {
        let Some(head) = self.active_head_id(session_id)? else {
            return Ok(Vec::new());
        };
        self.branch_to_leaf(session_id, &head)
    }

    pub fn branch_to_leaf(
        &self,
        session_id: &str,
        leaf_entry_id: &str,
    ) -> Result<Vec<SessionEntry>, ForestError> {
        self.ensure_session(session_id)?;
        let entries = store::session_entries(self.db, session_id)?;
        let by_id: HashMap<String, SessionEntry> = entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        if !by_id.contains_key(leaf_entry_id) {
            return Err(self.missing_leaf_error(session_id, leaf_entry_id)?);
        }
        let mut branch = Vec::new();
        let mut visited = HashSet::new();
        let mut current = Some(leaf_entry_id.to_owned());
        while let Some(entry_id) = current {
            if !visited.insert(entry_id.clone()) {
                return Err(ForestError::Cycle {
                    session_id: session_id.to_owned(),
                    entry_id,
                });
            }
            let entry = by_id
                .get(&entry_id)
                .cloned()
                .ok_or_else(|| ForestError::InvalidHead {
                    session_id: session_id.to_owned(),
                    entry_id: entry_id.clone(),
                })?;
            self.validate_stored_entry(&entry)?;
            current = match &entry.parent_entry_id {
                Some(parent_id) if by_id.contains_key(parent_id) => Some(parent_id.clone()),
                Some(parent_id) => {
                    return Err(self.parent_error(session_id, &entry.id, parent_id));
                }
                None => None,
            };
            branch.push(entry);
        }
        branch.reverse();
        Ok(branch)
    }

    pub fn children(
        &self,
        session_id: &str,
        parent_entry_id: Option<&str>,
    ) -> Result<Vec<SessionEntry>, ForestError> {
        self.ensure_session(session_id)?;
        if let Some(parent_entry_id) = parent_entry_id {
            self.ensure_entry_in_session(session_id, parent_entry_id)?;
        }
        let entries = store::session_entries(self.db, session_id)?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.parent_entry_id.as_deref() == parent_entry_id)
            .collect())
    }

    pub fn branch_leaves(&self, session_id: &str) -> Result<Vec<SessionEntry>, ForestError> {
        self.ensure_session(session_id)?;
        let entries = store::session_entries(self.db, session_id)?;
        let parent_ids: HashSet<String> = entries
            .iter()
            .filter_map(|entry| entry.parent_entry_id.clone())
            .collect();
        Ok(entries
            .into_iter()
            .filter(|entry| !parent_ids.contains(entry.id.as_str()))
            .collect())
    }

    pub fn integrity_report(&self) -> Result<IntegrityReport, ForestError> {
        let session_ids = {
            let mut statement = self.db.prepare("SELECT id FROM sessions ORDER BY rowid")?;
            let session_ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            session_ids
        };
        let mut report = IntegrityReport {
            healthy_sessions: Vec::new(),
            quarantined_sessions: Vec::new(),
        };
        for session_id in session_ids {
            let findings = self.session_findings(&session_id)?;
            if findings.is_empty() {
                report.healthy_sessions.push(session_id);
            } else {
                report.quarantined_sessions.push(QuarantinedSession {
                    session_id,
                    findings,
                });
            }
        }
        Ok(report)
    }

    fn session_findings(&self, session_id: &str) -> Result<Vec<ForestError>, ForestError> {
        let entries = store::session_entries(self.db, session_id)?;
        let by_id: HashMap<&str, &SessionEntry> = entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut findings = Vec::new();
        for entry in &entries {
            if let Err(error) = self.validate_stored_entry(entry) {
                findings.push(error);
            }
            if let Some(parent_id) = entry.parent_entry_id.as_deref() {
                if !by_id.contains_key(parent_id) {
                    findings.push(self.parent_error(session_id, &entry.id, parent_id));
                }
            }
        }
        let mut verified = HashSet::new();
        for entry in &entries {
            if verified.contains(entry.id.as_str()) {
                continue;
            }
            let mut path = Vec::new();
            let mut in_path = HashSet::new();
            let mut current = Some(entry.id.as_str());
            while let Some(entry_id) = current {
                if verified.contains(entry_id) {
                    break;
                }
                if !in_path.insert(entry_id) {
                    findings.push(ForestError::Cycle {
                        session_id: session_id.to_owned(),
                        entry_id: entry_id.to_owned(),
                    });
                    break;
                }
                path.push(entry_id);
                current = by_id
                    .get(entry_id)
                    .and_then(|entry| entry.parent_entry_id.as_deref())
                    .filter(|parent| by_id.contains_key(parent));
            }
            verified.extend(path);
        }
        if let Some(head_id) = self.raw_active_head_id(session_id)? {
            if !by_id.contains_key(head_id.as_str()) {
                findings.push(self.missing_leaf_error(session_id, &head_id)?);
            }
        }
        findings.sort_by_key(ToString::to_string);
        findings.dedup();
        Ok(findings)
    }

    fn validate_stored_entry(&self, entry: &SessionEntry) -> Result<(), ForestError> {
        match EntryKind::from_storage(&entry.kind) {
            EntryKind::Legacy(_) => {
                if entry.payload.is_object() {
                    Ok(())
                } else {
                    Err(ForestError::InvalidPayload {
                        kind: entry.kind.clone(),
                        reason: "legacy payload must remain inspectable as a JSON object".into(),
                    })
                }
            }
            kind => kind.validate_payload(&entry.payload),
        }
    }

    fn ensure_session(&self, session_id: &str) -> Result<(), ForestError> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        if exists {
            Ok(())
        } else {
            Err(ForestError::SessionNotFound(session_id.to_owned()))
        }
    }

    fn ensure_entry_in_session(&self, session_id: &str, entry_id: &str) -> Result<(), ForestError> {
        let actual_session: Option<String> = self
            .db
            .query_row(
                "SELECT session_id FROM session_entries WHERE id=?1",
                params![entry_id],
                |row| row.get(0),
            )
            .optional()?;
        match actual_session {
            Some(actual) if actual == session_id => Ok(()),
            Some(actual) => Err(ForestError::CrossSessionParent {
                session_id: session_id.to_owned(),
                entry_id: "new-entry".into(),
                parent_entry_id: entry_id.to_owned(),
                parent_session_id: actual,
            }),
            None => Err(ForestError::EntryNotFound {
                session_id: session_id.to_owned(),
                entry_id: entry_id.to_owned(),
            }),
        }
    }

    fn active_head_id(&self, session_id: &str) -> Result<Option<String>, ForestError> {
        self.ensure_session(session_id)?;
        let head = self.raw_active_head_id(session_id)?;
        if let Some(entry_id) = &head {
            self.ensure_entry_in_session(session_id, entry_id)
                .map_err(|_| ForestError::InvalidHead {
                    session_id: session_id.to_owned(),
                    entry_id: entry_id.clone(),
                })?;
        }
        Ok(head)
    }

    fn raw_active_head_id(&self, session_id: &str) -> Result<Option<String>, ForestError> {
        Ok(self
            .db
            .query_row(
                "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    fn parent_error(&self, session_id: &str, entry_id: &str, parent_entry_id: &str) -> ForestError {
        let parent_session = self
            .db
            .query_row(
                "SELECT session_id FROM session_entries WHERE id=?1",
                params![parent_entry_id],
                |row| row.get::<_, String>(0),
            )
            .optional();
        match parent_session {
            Ok(Some(parent_session_id)) => ForestError::CrossSessionParent {
                session_id: session_id.to_owned(),
                entry_id: entry_id.to_owned(),
                parent_entry_id: parent_entry_id.to_owned(),
                parent_session_id,
            },
            _ => ForestError::OrphanParent {
                entry_id: entry_id.to_owned(),
                parent_entry_id: parent_entry_id.to_owned(),
            },
        }
    }

    fn missing_leaf_error(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<ForestError, ForestError> {
        let actual_session = self
            .db
            .query_row(
                "SELECT session_id FROM session_entries WHERE id=?1",
                params![entry_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(match actual_session {
            Some(parent_session_id) => ForestError::CrossSessionParent {
                session_id: session_id.to_owned(),
                entry_id: entry_id.to_owned(),
                parent_entry_id: entry_id.to_owned(),
                parent_session_id,
            },
            None => ForestError::InvalidHead {
                session_id: session_id.to_owned(),
                entry_id: entry_id.to_owned(),
            },
        })
    }
}
