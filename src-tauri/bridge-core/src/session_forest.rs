use crate::{
    model::{
        SessionEntry, MIN_SUPPORTED_SEMANTIC_EVENT_SCHEMA_VERSION,
        SEMANTIC_EVENT_SCHEMA_VERSION,
    },
    store, BridgeError,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub(crate) const TYPED_SCHEMA_MARKER: &str = "_bridgeTypedSchemaVersion";
pub(crate) const TYPED_SCHEMA_VERSION: u64 = 1;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use serde_json::json;

    fn database(session_ids: &[&str]) -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/forest-demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/forest-workspace','idle','now')", []).unwrap();
        for session_id in session_ids {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES(?1,'w','codex','Codex','idle','reported')",
                params![session_id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES(?1,'fresh','now')",
                params![session_id],
            )
            .unwrap();
        }
        db
    }

    fn message(text: &str) -> Value {
        json!({ "text": text })
    }

    fn entry_ids(entries: &[SessionEntry]) -> Vec<String> {
        entries.iter().map(|entry| entry.id.clone()).collect()
    }

    #[test]
    fn all_entry_kinds_validate_their_payload_contract() {
        let cases = vec![
            (EntryKind::UserMessage, json!({"text":"hello"})),
            (EntryKind::AssistantMessage, json!({"text":"hello"})),
            (EntryKind::ToolStarted, json!({"toolId":"tool"})),
            (EntryKind::ToolCompleted, json!({"itemId":"tool"})),
            (
                EntryKind::ApprovalRequested,
                json!({"approvalId":"approval"}),
            ),
            (
                EntryKind::ApprovalResolved,
                json!({"approvalId":"approval"}),
            ),
            (
                EntryKind::DelegationRequested,
                json!({"requestId":"request"}),
            ),
            (
                EntryKind::DelegationApproved,
                json!({"requestId":"request"}),
            ),
            (
                EntryKind::DelegationRejected,
                json!({"requestId":"request"}),
            ),
            (EntryKind::WorkerResult, json!({"status":"completed"})),
            (
                EntryKind::Checkpoint,
                json!({"schemaVersion":1,"summary":"checkpoint"}),
            ),
            (EntryKind::CompactionRequested, json!({"reason":"pressure"})),
            (
                EntryKind::Compaction,
                json!({"schemaVersion":1,"summary":"compacted"}),
            ),
            (
                EntryKind::CompactionFailed,
                json!({"reason":"invalid output"}),
            ),
            (
                EntryKind::BranchSummary,
                json!({"summary":"alternate path"}),
            ),
            (EntryKind::ModelChanged, json!({"model":"runtime-model"})),
            (EntryKind::EffortChanged, json!({"effort":"high"})),
            (EntryKind::SessionStatus, json!({"status":"working"})),
            (
                EntryKind::SessionResumeFailed,
                json!({"reason":"thread expired"}),
            ),
            (EntryKind::ArtifactCreated, json!({"path":"result.json"})),
        ];
        for (kind, valid_payload) in cases {
            assert!(
                kind.validate_payload(&valid_payload).is_ok(),
                "{} rejected its valid payload",
                kind.as_str()
            );
            assert!(
                matches!(
                    kind.validate_payload(&json!({})),
                    Err(ForestError::InvalidPayload { .. })
                ),
                "{} accepted a payload missing required fields",
                kind.as_str()
            );
            assert_eq!(
                EntryKind::from_storage(kind.as_str()).as_str(),
                kind.as_str()
            );
        }
        assert!(EntryKind::SessionStatus
            .validate_payload(&json!({"status":"cancelled"}))
            .is_err());
        assert!(EntryKind::SessionStatus
            .validate_payload(&json!({"status":"cancelled","reason":"user_cancelled"}))
            .is_ok());
        assert!(matches!(
            EntryKind::Legacy("turn.completed".into()).validate_payload(&json!({})),
            Err(ForestError::UnsupportedEntryKind(_))
        ));
    }

    #[test]
    fn append_rewind_append_preserves_immutable_history() {
        let db = database(&["s"]);
        let forest = SessionForest::new(&db);
        let root = forest
            .append("s", EntryKind::UserMessage, message("root"))
            .unwrap();
        let mut snapshots = HashMap::from([(root.id.clone(), root)]);
        let mut insertion_ids = snapshots.keys().cloned().collect::<Vec<_>>();
        for index in 0..200 {
            if index > 0 && index % 7 == 0 {
                let rewind_index = (index * 13) % insertion_ids.len();
                forest
                    .move_head("s", Some(&insertion_ids[rewind_index]))
                    .unwrap();
            }
            let entry = forest
                .append(
                    "s",
                    EntryKind::AssistantMessage,
                    message(&format!("message-{index}")),
                )
                .unwrap();
            insertion_ids.push(entry.id.clone());
            snapshots.insert(entry.id.clone(), entry);
        }
        let stored = store::session_entries(&db, "s").unwrap();
        assert_eq!(stored.len(), snapshots.len());
        assert_eq!(
            stored
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            (1..=stored.len() as i64).collect::<Vec<_>>()
        );
        for entry in stored {
            assert_eq!(Some(&entry), snapshots.get(&entry.id));
        }
        assert!(forest.branch_leaves("s").unwrap().len() > 1);
    }

    #[test]
    fn active_branch_traversal_is_deterministic() {
        let db = database(&["s"]);
        let forest = SessionForest::new(&db);
        forest
            .append("s", EntryKind::UserMessage, message("root"))
            .unwrap();
        for index in 0..25 {
            forest
                .append(
                    "s",
                    EntryKind::AssistantMessage,
                    message(&format!("{index}")),
                )
                .unwrap();
        }
        let first = entry_ids(&forest.active_branch("s").unwrap());
        let second = entry_ids(&forest.active_branch("s").unwrap());
        assert_eq!(first, second);
        assert_eq!(first.len(), 26);
    }

    #[test]
    fn forks_share_a_prefix_and_diverge_after_the_fork_point() {
        let db = database(&["s"]);
        let forest = SessionForest::new(&db);
        let root = forest
            .append("s", EntryKind::UserMessage, message("root"))
            .unwrap();
        let fork_point = forest
            .append("s", EntryKind::AssistantMessage, message("common"))
            .unwrap();
        let left = forest
            .append("s", EntryKind::AssistantMessage, message("left"))
            .unwrap();
        forest.move_head("s", Some(&fork_point.id)).unwrap();
        let right = forest
            .append("s", EntryKind::AssistantMessage, message("right"))
            .unwrap();
        let right_tail = forest
            .append("s", EntryKind::AssistantMessage, message("right-tail"))
            .unwrap();

        assert_eq!(
            entry_ids(&forest.branch_to_leaf("s", &left.id).unwrap()),
            vec![root.id.clone(), fork_point.id.clone(), left.id.clone()]
        );
        assert_eq!(
            entry_ids(&forest.branch_to_leaf("s", &right_tail.id).unwrap()),
            vec![
                root.id,
                fork_point.id.clone(),
                right.id.clone(),
                right_tail.id.clone()
            ]
        );
        assert_eq!(
            entry_ids(&forest.children("s", Some(&fork_point.id)).unwrap()),
            vec![left.id.clone(), right.id]
        );
        assert_eq!(
            entry_ids(&forest.branch_leaves("s").unwrap()),
            vec![left.id, right_tail.id]
        );
    }

    #[test]
    fn branch_summary_is_an_immutable_entry() {
        let db = database(&["s"]);
        let forest = SessionForest::new(&db);
        let root = forest
            .append("s", EntryKind::UserMessage, message("root"))
            .unwrap();
        let summary = forest
            .append_branch_summary("s", Some(&root.id), "Explored the alternate")
            .unwrap();
        assert_eq!(summary.kind, "branch.summary");
        assert_eq!(summary.payload["summary"], "Explored the alternate");
        assert_eq!(summary.parent_entry_id.as_deref(), Some(&*root.id));
        assert_eq!(
            entry_ids(&forest.active_branch("s").unwrap()),
            vec![root.id, summary.id]
        );
    }

    #[test]
    fn traverses_ten_thousand_entries_without_recursion() {
        let db = database(&["s"]);
        let transaction = db.unchecked_transaction().unwrap();
        let mut parent: Option<String> = None;
        for index in 0..10_000 {
            let id = format!("entry-{index:05}");
            transaction
                .execute(
                    "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
                     VALUES(?1,'s',?2,?3,'assistant.message',?4,'now')",
                    params![id, parent, index + 1, message(&index.to_string()).to_string()],
                )
                .unwrap();
            parent = Some(id);
        }
        transaction
            .execute(
                "UPDATE session_heads SET active_entry_id=?1 WHERE session_id='s'",
                params![parent],
            )
            .unwrap();
        transaction.commit().unwrap();
        let branch = SessionForest::new(&db).active_branch("s").unwrap();
        assert_eq!(branch.len(), 10_000);
        assert_eq!(branch.first().unwrap().id, "entry-00000");
        assert_eq!(branch.last().unwrap().id, "entry-09999");
    }

    #[test]
    fn detects_and_quarantines_corruption_without_breaking_siblings() {
        let db = database(&["healthy", "orphan", "cross", "cycle", "bad-head"]);
        let forest = SessionForest::new(&db);
        let healthy = forest
            .append("healthy", EntryKind::UserMessage, message("healthy"))
            .unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES('orphan-entry','orphan','missing-parent',1,'user.message','{\"text\":\"orphan\"}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES('cross-entry','cross',?1,1,'user.message','{\"text\":\"cross\"}','now')",
            params![healthy.id],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES('cycle-a','cycle','cycle-b',1,'user.message','{\"text\":\"a\"}','now');
             INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at)
             VALUES('cycle-b','cycle','cycle-a',2,'user.message','{\"text\":\"b\"}','now');
             UPDATE session_heads SET active_entry_id='orphan-entry' WHERE session_id='orphan';
             UPDATE session_heads SET active_entry_id='cross-entry' WHERE session_id='cross';
             UPDATE session_heads SET active_entry_id='cycle-a' WHERE session_id='cycle';
             UPDATE session_heads SET active_entry_id='missing-head' WHERE session_id='bad-head';
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();

        let report = forest.integrity_report().unwrap();
        assert_eq!(report.healthy_sessions, vec!["healthy"]);
        assert_eq!(
            report
                .quarantined_sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["orphan", "cross", "cycle", "bad-head"]
        );
        assert!(report.quarantined_sessions.iter().any(|session| session
            .findings
            .iter()
            .any(|finding| matches!(finding, ForestError::OrphanParent { .. }))));
        assert!(report.quarantined_sessions.iter().any(|session| session
            .findings
            .iter()
            .any(|finding| matches!(finding, ForestError::CrossSessionParent { .. }))));
        assert!(report.quarantined_sessions.iter().any(|session| session
            .findings
            .iter()
            .any(|finding| matches!(finding, ForestError::Cycle { .. }))));
        assert!(report.quarantined_sessions.iter().any(|session| session
            .findings
            .iter()
            .any(|finding| matches!(finding, ForestError::InvalidHead { .. }))));
        assert_eq!(forest.active_branch("healthy").unwrap().len(), 1);
        assert!(matches!(
            forest.active_branch("orphan"),
            Err(ForestError::OrphanParent { .. })
        ));
    }

    #[test]
    fn public_navigation_never_mutates_entries() {
        let db = database(&["s"]);
        let forest = SessionForest::new(&db);
        let root = forest
            .append("s", EntryKind::UserMessage, message("root"))
            .unwrap();
        let leaf = forest
            .append("s", EntryKind::AssistantMessage, message("leaf"))
            .unwrap();
        let before = store::session_entries(&db, "s").unwrap();
        forest.move_head("s", Some(&root.id)).unwrap();
        forest.move_head("s", None).unwrap();
        forest.move_head("s", Some(&leaf.id)).unwrap();
        let after = store::session_entries(&db, "s").unwrap();
        assert_eq!(before, after);
        assert_eq!(forest.active_branch("s").unwrap(), after);
    }

    #[test]
    fn compatibility_kinds_are_readable_but_not_appendable() {
        let db = database(&["s"]);
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             VALUES('legacy','s',1,'turn.completed','{\"usage\":{\"input\":1}}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE session_heads SET active_entry_id='legacy' WHERE session_id='s'",
            [],
        )
        .unwrap();
        let forest = SessionForest::new(&db);
        assert_eq!(forest.active_branch("s").unwrap()[0].kind, "turn.completed");
        assert!(matches!(
            forest.append("s", EntryKind::Legacy("turn.completed".into()), json!({})),
            Err(ForestError::UnsupportedEntryKind(_))
        ));
    }

    #[test]
    fn compatibility_kind_collision_remains_readable() {
        let db = database(&["s"]);
        let event = crate::agent::NormalizedEvent {
            kind: "approval.resolved".into(),
            item_id: None,
            role: Some("system".into()),
            status: Some("completed".into()),
            title: Some("Approval resolved".into()),
            text: None,
            data: json!({"decision":"accept"}),
        };
        store::session_event(&db, "s", &event, &json!({"provider":"claude"})).unwrap();
        let forest = SessionForest::new(&db);
        let branch = forest.active_branch("s").unwrap();
        assert_eq!(branch.len(), 1);
        assert_eq!(branch[0].kind, "approval.resolved");
        assert_eq!(branch[0].payload["itemId"], Value::Null);
        assert_eq!(
            forest.integrity_report().unwrap().healthy_sessions,
            vec!["s"]
        );
    }

    #[test]
    fn invalid_typed_payload_still_fails_traversal() {
        let db = database(&["s"]);
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             VALUES('invalid-typed','s',1,'approval.resolved','{\"_bridgeTypedSchemaVersion\":1}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE session_heads SET active_entry_id='invalid-typed' WHERE session_id='s'",
            [],
        )
        .unwrap();
        let forest = SessionForest::new(&db);
        assert!(matches!(
            forest.active_branch("s"),
            Err(ForestError::InvalidPayload { .. })
        ));
        assert_eq!(
            forest.integrity_report().unwrap().quarantined_sessions[0].session_id,
            "s"
        );
    }

    #[test]
    fn unsupported_future_semantic_schema_fails_closed() {
        let db = database(&["s"]);
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,semantic_schema_version,kind,payload,created_at)
             VALUES('future','s',1,3,'user.message','{\"text\":\"future\"}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE session_heads SET active_entry_id='future' WHERE session_id='s'",
            [],
        )
        .unwrap();
        assert!(matches!(
            SessionForest::new(&db).active_branch("s"),
            Err(ForestError::UnsupportedSemanticSchemaVersion { version: 3, .. })
        ));
    }

    #[test]
    fn append_validates_parent_directly_after_compatibility_history() {
        let db = database(&["s"]);
        let event = crate::agent::NormalizedEvent {
            kind: "approval.resolved".into(),
            item_id: None,
            role: Some("system".into()),
            status: Some("completed".into()),
            title: None,
            text: None,
            data: json!({"decision":"accept"}),
        };
        store::session_event(&db, "s", &event, &json!({})).unwrap();
        let forest = SessionForest::new(&db);
        let typed = forest
            .append("s", EntryKind::AssistantMessage, message("continued"))
            .unwrap();
        assert_eq!(typed.sequence, 2);
        assert_eq!(typed.payload[TYPED_SCHEMA_MARKER], TYPED_SCHEMA_VERSION);
        assert_eq!(forest.active_branch("s").unwrap().len(), 2);
    }
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
    #[error("entry {entry_id} belongs to session {actual_session_id}, not {expected_session_id}")]
    EntryBelongsToOtherSession {
        entry_id: String,
        expected_session_id: String,
        actual_session_id: String,
    },
    #[error("unsupported entry kind for append: {0}")]
    UnsupportedEntryKind(String),
    #[error("unsupported semantic event schema version {version} on entry {entry_id}")]
    UnsupportedSemanticSchemaVersion { entry_id: String, version: i64 },
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
            self.ensure_entry_in_session(session_id, parent_entry_id)?;
        }
        let payload = mark_typed_payload(payload);
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

    pub(crate) fn validate_stored_entry(&self, entry: &SessionEntry) -> Result<(), ForestError> {
        if !(MIN_SUPPORTED_SEMANTIC_EVENT_SCHEMA_VERSION..=SEMANTIC_EVENT_SCHEMA_VERSION)
            .contains(&entry.semantic_schema_version)
        {
            return Err(ForestError::UnsupportedSemanticSchemaVersion {
                entry_id: entry.id.clone(),
                version: entry.semantic_schema_version,
            });
        }
        if !entry.payload.is_object() {
            return Err(ForestError::InvalidPayload {
                kind: entry.kind.clone(),
                reason: "stored payload must remain inspectable as a JSON object".into(),
            });
        }
        if entry
            .payload
            .get(TYPED_SCHEMA_MARKER)
            .and_then(Value::as_u64)
            == Some(TYPED_SCHEMA_VERSION)
        {
            EntryKind::from_storage(&entry.kind).validate_payload(&entry.payload)
        } else {
            // Migration/dual-write compatibility entries preserve normalized
            // provider payloads, whose shapes predate typed forest contracts.
            Ok(())
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
            Some(actual) => Err(ForestError::EntryBelongsToOtherSession {
                entry_id: entry_id.to_owned(),
                expected_session_id: session_id.to_owned(),
                actual_session_id: actual,
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

pub(crate) fn append_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &str,
    kind: EntryKind,
    payload: Value,
) -> Result<SessionEntry, ForestError> {
    kind.validate_payload(&payload)?;
    let session_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
        params![session_id],
        |row| row.get(0),
    )?;
    if !session_exists {
        return Err(ForestError::SessionNotFound(session_id.to_owned()));
    }
    let parent_entry_id: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(parent_entry_id) = &parent_entry_id {
        let parent_session: Option<String> = transaction
            .query_row(
                "SELECT session_id FROM session_entries WHERE id=?1",
                params![parent_entry_id],
                |row| row.get(0),
            )
            .optional()?;
        if parent_session.as_deref() != Some(session_id) {
            return Err(ForestError::InvalidHead {
                session_id: session_id.to_owned(),
                entry_id: parent_entry_id.clone(),
            });
        }
    }
    Ok(store::append_session_entry_tx(
        transaction,
        session_id,
        parent_entry_id.as_deref(),
        kind.as_str(),
        &mark_typed_payload(payload),
        None,
        "eligible",
        None,
    )?)
}

fn mark_typed_payload(mut payload: Value) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            TYPED_SCHEMA_MARKER.into(),
            Value::from(TYPED_SCHEMA_VERSION),
        );
    }
    payload
}
