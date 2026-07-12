use crate::{
    model::SessionEntry,
    session_forest::{self, EntryKind},
    worker_lifecycle::{validate_transition, WorkerLifecycleState},
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::str::FromStr;

pub struct SessionSupervisor;

impl SessionSupervisor {
    pub fn transition(
        db: &Connection,
        session_id: &str,
        next: WorkerLifecycleState,
        reason: Option<&str>,
    ) -> Result<SessionEntry, BridgeError> {
        let transaction = db.unchecked_transaction()?;
        let current: String = transaction
            .query_row(
                "SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| BridgeError::Invalid(format!("worker runtime {session_id} not found")))?;
        let current = WorkerLifecycleState::from_str(&current)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        validate_transition(current, next).map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        let mut payload = serde_json::json!({"status": next.as_str(), "from": current.as_str()});
        if let Some(reason) = reason {
            payload["reason"] = serde_json::Value::String(reason.to_owned());
        }
        let entry = session_forest::append_in_transaction(
            &transaction,
            session_id,
            EntryKind::SessionStatus,
            payload,
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        transaction.execute(
            "UPDATE worker_runtime SET lifecycle_state=?2,updated_at=?3 WHERE session_id=?1",
            params![session_id, next.as_str(), now],
        )?;
        transaction.execute(
            "UPDATE sessions SET status=?2,ended_at=CASE WHEN ?2 IN ('completed','cancelled') THEN ?3 ELSE ended_at END WHERE id=?1",
            params![session_id, next.as_str(), now],
        )?;
        transaction.execute(
            "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor','worker.lifecycle',?1,?2,?3)",
            params![session_id, format!("{} -> {}", current.as_str(), next.as_str()), now],
        )?;
        transaction.commit()?;
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::WorkerRuntimeRecord, store};

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/supervisor','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/supervisor-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','starting','reported','parent',1)", []).unwrap();
        store::upsert_worker_runtime(&db, &WorkerRuntimeRecord { session_id:"child".into(), parent_session_id:"parent".into(), lifecycle_state:"starting".into(), task_family:"implementation".into(), compatibility_key:"key".into(), result_status:"pending".into(), retry_count:0, warm_until:None, last_result:None, updated_at:"now".into() }).unwrap();
        db
    }

    #[test]
    fn transition_persists_runtime_session_forest_and_audit_together() {
        let db = database();
        let entry = SessionSupervisor::transition(
            &db,
            "child",
            WorkerLifecycleState::Working,
            Some("provider_started"),
        )
        .unwrap();
        assert_eq!(entry.kind, "session.status");
        assert_eq!(entry.payload["status"], "working");
        assert_eq!(store::worker_runtime(&db, "child").unwrap().unwrap().lifecycle_state, "working");
        let status: String = db.query_row("SELECT status FROM sessions WHERE id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(status, "working");
        let audit: i64 = db.query_row("SELECT COUNT(*) FROM events WHERE entity_id='child' AND kind='worker.lifecycle'", [], |row| row.get(0)).unwrap();
        assert_eq!(audit, 1);
    }

    #[test]
    fn illegal_transition_changes_nothing() {
        let db = database();
        assert!(SessionSupervisor::transition(
            &db,
            "child",
            WorkerLifecycleState::Completed,
            None,
        )
        .is_err());
        assert_eq!(store::worker_runtime(&db, "child").unwrap().unwrap().lifecycle_state, "starting");
        assert!(store::session_entries(&db, "child").unwrap().is_empty());
    }
}
