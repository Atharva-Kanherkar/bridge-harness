use crate::{
    delegation::{SuggestedNextAction, WorkerResult, WorkerResultStatus},
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

    pub fn record_result(
        db: &Connection,
        session_id: &str,
        result: &WorkerResult,
    ) -> Result<Option<String>, BridgeError> {
        result.validate().map_err(BridgeError::Invalid)?;
        let transaction = db.unchecked_transaction()?;
        let runtime: Option<(String, String)> = transaction
            .query_row(
                "SELECT parent_session_id,result_status FROM worker_runtime WHERE session_id=?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((parent_session_id, result_status)) = runtime else {
            return Ok(None);
        };
        if result_status == "reported" {
            return Ok(None);
        }
        let mut payload = serde_json::to_value(result)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        payload["childSessionId"] = serde_json::Value::String(session_id.to_owned());
        session_forest::append_in_transaction(
            &transaction,
            session_id,
            EntryKind::WorkerResult,
            payload.clone(),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        session_forest::append_in_transaction(
            &transaction,
            &parent_session_id,
            EntryKind::WorkerResult,
            payload.clone(),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE worker_runtime SET result_status='reported',last_result=?2,updated_at=?3 WHERE session_id=?1",
            params![session_id, payload.to_string(), now],
        )?;
        transaction.execute(
            "UPDATE worker_leases SET lease_status=CASE
                WHEN (SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1)='warm' THEN 'warm'
                WHEN (SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1)='stopped' THEN 'checkpointed'
                ELSE 'released' END,updated_at=?2 WHERE session_id=?1",
            params![session_id, now],
        )?;
        let remaining: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM worker_runtime WHERE parent_session_id=?1 AND result_status!='reported'",
            params![parent_session_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            transaction.execute(
                "UPDATE sessions SET status='ready' WHERE id=?1 AND status='waiting'",
                params![parent_session_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor','worker.result.reported',?1,?2,?3)",
            params![session_id, result.status.as_str(), now],
        )?;
        transaction.commit()?;
        Ok(Some(parent_session_id))
    }

    pub fn recover_orphaned_workers(db: &Connection) -> Result<usize, BridgeError> {
        let workers = {
            let mut statement = db.prepare(
                "SELECT session_id,lifecycle_state,result_status FROM worker_runtime
                 WHERE lifecycle_state NOT IN ('completed','cancelled','stopped') OR result_status!='reported'
                 ORDER BY updated_at,session_id",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut reconciled = 0;
        for (session_id, state, result_status) in workers {
            let state = WorkerLifecycleState::from_str(&state)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?;
            for next in recovery_path(state) {
                Self::transition(db, &session_id, next, Some("app_restart"))?;
            }
            if result_status != "reported" {
                let cancelled = state == WorkerLifecycleState::Cancelled;
                let result = WorkerResult {
                    schema_version: crate::delegation::SCHEMA_VERSION,
                    status: if cancelled { WorkerResultStatus::Cancelled } else { WorkerResultStatus::Failed },
                    summary: if cancelled { "Worker cancellation was recovered after Bridge restarted".into() } else { "Worker process ended when Bridge restarted".into() },
                    files_changed: vec![],
                    tests: vec![],
                    decisions: vec![],
                    risks: vec!["The provider process was not alive during restart recovery".into()],
                    remaining_work: vec!["Resume a compatible worker or delegate again".into()],
                    suggested_next_action: SuggestedNextAction::Finish,
                    suggested_role: None,
                    suggested_task: None,
                };
                Self::record_result(db, &session_id, &result)?;
            }
            reconciled += 1;
        }
        Ok(reconciled)
    }
}

fn recovery_path(state: WorkerLifecycleState) -> Vec<WorkerLifecycleState> {
    use WorkerLifecycleState::*;
    match state {
        Starting => vec![Working, Checkpointing, Stopped],
        Working => vec![Checkpointing, Stopped],
        Waiting => vec![Working, Checkpointing, Stopped],
        Warm => vec![Checkpointing, Stopped],
        Checkpointing => vec![Stopped],
        Resuming => vec![Restored, Working, Checkpointing, Stopped],
        Restored => vec![Working, Checkpointing, Stopped],
        Failed => vec![Completed],
        Stopped | Completed | Cancelled => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::{SuggestedNextAction, WorkerResultStatus},
        model::WorkerRuntimeRecord,
        store,
    };

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/supervisor','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/supervisor-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','starting','reported','parent',1)", []).unwrap();
        store::upsert_worker_runtime(&db, &WorkerRuntimeRecord { session_id:"child".into(), parent_session_id:"parent".into(), lifecycle_state:"starting".into(), task_family:"implementation".into(), compatibility_key:"key".into(), result_status:"pending".into(), retry_count:0, warm_until:None, worktree_path:None, worktree_branch:None, last_result:None, updated_at:"now".into() }).unwrap();
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

    #[test]
    fn typed_result_is_reported_and_releases_parent_exactly_once() {
        let db = database();
        db.execute("UPDATE sessions SET status='waiting' WHERE id='parent'", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,write_mode,lease_status,created_at,updated_at) VALUES('child','w','implementation','standard','implementation','shared','active','now','now')", []).unwrap();
        let result = WorkerResult {
            schema_version: 1,
            status: WorkerResultStatus::Completed,
            summary: "Implemented the change".into(),
            files_changed: vec![],
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        assert_eq!(
            SessionSupervisor::record_result(&db, "child", &result).unwrap(),
            Some("parent".into())
        );
        assert_eq!(SessionSupervisor::record_result(&db, "child", &result).unwrap(), None);
        assert_eq!(store::outstanding_children(&db, "parent").unwrap(), 0);
        assert_eq!(store::session_entries(&db, "child").unwrap().last().unwrap().kind, "worker.result");
        assert_eq!(store::session_entries(&db, "parent").unwrap().last().unwrap().payload["status"], "completed");
        let (parent_status, lease_status): (String, String) = (
            db.query_row("SELECT status FROM sessions WHERE id='parent'", [], |row| row.get(0)).unwrap(),
            db.query_row("SELECT lease_status FROM worker_leases WHERE session_id='child'", [], |row| row.get(0)).unwrap(),
        );
        assert_eq!((parent_status.as_str(), lease_status.as_str()), ("ready", "released"));
    }

    #[test]
    fn restart_reconciles_working_waiting_and_warm_workers_without_orphans() {
        let db = database();
        db.execute("UPDATE sessions SET status='waiting' WHERE id='parent'", []).unwrap();
        db.execute("UPDATE sessions SET status='working' WHERE id='child'", []).unwrap();
        db.execute("UPDATE worker_runtime SET lifecycle_state='working' WHERE session_id='child'", []).unwrap();
        for (session_id, lifecycle) in [("waiting-child", "waiting"), ("warm-child", "warm")] {
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES(?1,'w','claude','Worker',?2,'reported','parent',1)", params![session_id,lifecycle]).unwrap();
            store::upsert_worker_runtime(&db, &WorkerRuntimeRecord { session_id:session_id.into(), parent_session_id:"parent".into(), lifecycle_state:lifecycle.into(), task_family:"implementation".into(), compatibility_key:format!("key-{session_id}"), result_status:"pending".into(), retry_count:0, warm_until:None, worktree_path:None, worktree_branch:None, last_result:None, updated_at:"now".into() }).unwrap();
        }
        for session_id in ["child", "waiting-child", "warm-child"] {
            db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,write_mode,lease_status,created_at,updated_at) VALUES(?1,'w','implementation','standard','implementation','shared','active','now','now')", params![session_id]).unwrap();
        }

        assert_eq!(SessionSupervisor::recover_orphaned_workers(&db).unwrap(), 3);
        assert_eq!(store::outstanding_children(&db, "parent").unwrap(), 0);
        let parent_status: String = db.query_row("SELECT status FROM sessions WHERE id='parent'", [], |row| row.get(0)).unwrap();
        assert_eq!(parent_status, "ready");
        for session_id in ["child", "waiting-child", "warm-child"] {
            let runtime = store::worker_runtime(&db, session_id).unwrap().unwrap();
            assert_eq!((runtime.lifecycle_state.as_str(), runtime.result_status.as_str()), ("stopped", "reported"));
            let lease: String = db.query_row("SELECT lease_status FROM worker_leases WHERE session_id=?1", params![session_id], |row| row.get(0)).unwrap();
            assert_eq!(lease, "checkpointed");
            let entries = store::session_entries(&db, session_id).unwrap();
            assert!(entries.iter().any(|entry| entry.kind == "session.status"));
            assert_eq!(entries.last().unwrap().kind, "worker.result");
        }
        let parent_results = store::session_entries(&db, "parent").unwrap().into_iter().filter(|entry| entry.kind == "worker.result").count();
        assert_eq!(parent_results, 3);
    }

    #[test]
    fn cancellation_releases_lease_reports_parent_and_never_increments_retry() {
        let db = database();
        db.execute("UPDATE sessions SET status='waiting' WHERE id='parent'", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,write_mode,lease_status,created_at,updated_at) VALUES('child','w','implementation','standard','implementation','shared','active','now','now')", []).unwrap();
        SessionSupervisor::transition(&db, "child", WorkerLifecycleState::Working, Some("provider_started")).unwrap();
        SessionSupervisor::transition(&db, "child", WorkerLifecycleState::Cancelled, Some("user_cancelled")).unwrap();
        let result = WorkerResult {
            schema_version: crate::delegation::SCHEMA_VERSION,
            status: WorkerResultStatus::Cancelled,
            summary: "Worker cancelled by user".into(),
            files_changed: vec![], tests: vec![], decisions: vec![], risks: vec![], remaining_work: vec!["Cancelled work was not completed".into()],
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: None, suggested_task: None,
        };
        assert_eq!(SessionSupervisor::record_result(&db, "child", &result).unwrap(), Some("parent".into()));
        let runtime = store::worker_runtime(&db, "child").unwrap().unwrap();
        assert_eq!((runtime.lifecycle_state.as_str(), runtime.result_status.as_str(), runtime.retry_count), ("cancelled", "reported", 0));
        assert_eq!(runtime.last_result.unwrap()["status"], "cancelled");
        let lease: String = db.query_row("SELECT lease_status FROM worker_leases WHERE session_id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(lease, "released");
        assert_eq!(store::outstanding_children(&db, "parent").unwrap(), 0);
    }
}
