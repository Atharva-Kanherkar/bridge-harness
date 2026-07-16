use crate::{
    adapters,
    delegation::{SuggestedNextAction, WorkerEvidence, WorkerResult, WorkerResultStatus, MAX_EVIDENCE_REFERENCES},
    completion, learning_router,
    model::SessionEntry,
    session_forest::{self, EntryKind},
    worker_lifecycle::{validate_transition, WorkerLifecycleState},
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::{collections::HashSet, str::FromStr};

pub struct SessionSupervisor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedWorkerResult {
    pub parent_session_id: String,
    pub evidence_id: String,
}

impl SessionSupervisor {
    pub fn track_adapter_process(
        db: &Connection,
        session_id: &str,
        pid: u32,
    ) -> Result<String, BridgeError> {
        let identity = adapters::process_identity(pid).ok_or_else(|| {
            BridgeError::Adapter(format!("provider process {pid} has no verifiable OS identity"))
        })?;
        let updated = db.execute(
            "UPDATE sessions SET adapter_pid=?2,adapter_process_identity=?3 WHERE id=?1",
            params![session_id, i64::from(pid), identity],
        )?;
        if updated != 1 {
            return Err(BridgeError::Invalid(format!("session {session_id} not found while tracking provider process")));
        }
        Ok(identity)
    }

    pub fn clear_adapter_process(db: &Connection, session_id: &str) -> Result<(), BridgeError> {
        db.execute(
            "UPDATE sessions SET adapter_pid=NULL,adapter_process_identity=NULL WHERE id=?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn recover_tracked_adapter_processes(db: &Connection) -> Result<usize, BridgeError> {
        let claims = {
            let mut statement = db.prepare(
                "SELECT id,status,adapter_pid,adapter_process_identity,active_turn_id IS NOT NULL
                 FROM sessions WHERE adapter_pid IS NOT NULL ORDER BY started_at,id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,bool>(4)?))
            })?.collect::<Result<Vec<_>,_>>()?;
            rows
        };
        let mut recovered = 0;
        for (session_id, status, raw_pid, expected_identity, was_mid_turn) in claims {
            let pid = u32::try_from(raw_pid).map_err(|_| BridgeError::Invalid(format!("session {session_id} has invalid provider PID {raw_pid}")))?;
            let live_identity = adapters::process_identity(pid);
            let identity_matches = expected_identity.as_ref().zip(live_identity.as_ref()).is_some_and(|(expected,live)| expected == live);
            let recovery_kind = if identity_matches {
                if !adapters::terminate_process_group(pid) {
                    return Err(BridgeError::Adapter(format!("tracked orphan process group {pid} could not be terminated")));
                }
                "adapter.orphan_killed"
            } else if live_identity.is_some() {
                "adapter.orphan_identity_mismatch"
            } else {
                "adapter.orphan_missing"
            };
            let active = !matches!(status.as_str(), "completed" | "cancelled" | "stopped" | "failed");
            let transaction = db.unchecked_transaction()?;
            transaction.execute("UPDATE sessions SET adapter_pid=NULL,adapter_process_identity=NULL WHERE id=?1", params![session_id])?;
            let now = Utc::now().to_rfc3339();
            let warning = if was_mid_turn {
                "Bridge restarted during an active provider turn; worktree changes may be partial and no checkpoint is implied"
            } else {
                "Bridge restarted with a tracked provider process; restoration is required before continuation"
            };
            if active {
                session_forest::append_in_transaction(
                    &transaction,
                    &session_id,
                    EntryKind::SessionStatus,
                    serde_json::json!({"status":"failed","reason":"supervisor_restart_orphan","recoverable":true,"warning":warning}),
                ).map_err(|error| BridgeError::Invalid(error.to_string()))?;
                transaction.execute("UPDATE sessions SET status='failed',active_turn_id=NULL,ended_at=?2 WHERE id=?1", params![session_id,now])?;
                transaction.execute("INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('adapter','adapter.request_failed',?1,?2,?3)", params![session_id,warning,now])?;
            }
            transaction.execute("INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor',?2,?1,?3,?4)", params![session_id,recovery_kind,warning,now])?;
            transaction.commit()?;
            recovered += 1;
        }
        Ok(recovered)
    }

    pub fn reconcile_workspace_statuses(db: &Connection) -> Result<(), BridgeError> {
        db.execute_batch(
            "UPDATE workspaces SET status=CASE
                WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
                WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status IN ('starting','working','warm','checkpointing','resuming','restored')) THEN 'working'
                WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='failed') THEN 'failed'
                ELSE 'ready' END;",
        )?;
        Ok(())
    }

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
    ) -> Result<Option<ReportedWorkerResult>, BridgeError> {
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
        let parent_entry = session_forest::append_in_transaction(
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
        if remaining == 0 && completion::completion_allows_ready(&transaction, &parent_session_id)? {
            transaction.execute(
                "UPDATE sessions SET status='ready' WHERE id=?1 AND status='waiting'",
                params![parent_session_id],
            )?;
        }
        transaction.execute(
            "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor','worker.result.reported',?1,?2,?3)",
            params![session_id, result.status.as_str(), now],
        )?;
        learning_router::record_worker_outcome(&transaction, session_id, result)?;
        transaction.commit()?;
        Ok(Some(ReportedWorkerResult {
            parent_session_id,
            evidence_id: parent_entry.id,
        }))
    }

    pub fn worker_evidence(
        db: &Connection,
        parent_session_id: &str,
        requested_ids: &[String],
    ) -> Result<Vec<WorkerEvidence>, BridgeError> {
        if requested_ids.len() > MAX_EVIDENCE_REFERENCES {
            return Err(BridgeError::Invalid(format!("worker evidence cannot contain more than {MAX_EVIDENCE_REFERENCES} references")));
        }
        if requested_ids.iter().collect::<HashSet<_>>().len() != requested_ids.len() {
            return Err(BridgeError::Invalid("worker evidence references cannot contain duplicates".into()));
        }
        let branch = session_forest::SessionForest::new(db)
            .active_branch(parent_session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let available = branch.into_iter().filter(|entry| entry.kind == EntryKind::WorkerResult.as_str()).collect::<Vec<_>>();
        let selected = if requested_ids.is_empty() {
            available.iter().rev().take(MAX_EVIDENCE_REFERENCES).rev().collect::<Vec<_>>()
        } else {
            requested_ids.iter().map(|id| {
                available.iter().find(|entry| entry.id == *id).ok_or_else(|| BridgeError::Invalid(format!("worker evidence {id} is not on the active parent branch")))
            }).collect::<Result<Vec<_>, _>>()?
        };
        selected.into_iter().map(|entry| {
            let child_session_id = entry.payload.get("childSessionId")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| BridgeError::Invalid(format!("worker evidence {} has no child session", entry.id)))?
                .to_owned();
            let mut payload = entry.payload.clone();
            let object = payload.as_object_mut().ok_or_else(|| BridgeError::Invalid(format!("worker evidence {} is not an object", entry.id)))?;
            object.remove("childSessionId");
            object.remove("_bridgeTypedSchemaVersion");
            object.remove("_bridgeRepoState");
            let result: WorkerResult = serde_json::from_value(payload)
                .map_err(|error| BridgeError::Invalid(format!("worker evidence {} is malformed: {error}", entry.id)))?;
            result.validate().map_err(BridgeError::Invalid)?;
            Ok(WorkerEvidence { evidence_id: entry.id.clone(), child_session_id, result })
        }).collect()
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
                    risks: vec!["The provider process was terminated or missing during restart recovery; worktree changes may be partial and no checkpoint is implied".into()],
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
    #[cfg(unix)]
    use std::process::Command;

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/supervisor','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/supervisor-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','starting','reported','parent',1)", []).unwrap();
        store::upsert_worker_runtime(&db, &WorkerRuntimeRecord { session_id:"child".into(), parent_session_id:"parent".into(), lifecycle_state:"starting".into(), task_family:"implementation".into(), compatibility_key:"key".into(), result_status:"pending".into(), retry_count:0, warm_until:None, worktree_path:None, worktree_branch:None, last_result:None, updated_at:"now".into() }).unwrap();
        db
    }

    #[cfg(unix)]
    fn sleeping_process() -> std::process::Child {
        let mut command = Command::new("sleep");
        command.arg("30");
        adapters::configure_process_group(&mut command);
        command.spawn().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn restart_kills_matching_worker_process_then_reports_typed_recoverable_failure() {
        let db = database();
        db.execute("UPDATE sessions SET status='working',active_turn_id='turn' WHERE id='child'", []).unwrap();
        db.execute("UPDATE worker_runtime SET lifecycle_state='working' WHERE session_id='child'", []).unwrap();
        let mut child = sleeping_process();
        let pid = child.id();
        SessionSupervisor::track_adapter_process(&db, "child", pid).unwrap();

        assert_eq!(SessionSupervisor::recover_tracked_adapter_processes(&db).unwrap(), 1);
        let _ = child.wait();
        assert!(adapters::process_identity(pid).is_none());
        let tracked: (Option<i64>,Option<String>) = db.query_row("SELECT adapter_pid,adapter_process_identity FROM sessions WHERE id='child'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(tracked, (None, None));
        assert_eq!(SessionSupervisor::recover_orphaned_workers(&db).unwrap(), 1);
        let runtime = crate::store::worker_runtime(&db, "child").unwrap().unwrap();
        assert_eq!((runtime.lifecycle_state.as_str(),runtime.result_status.as_str()), ("stopped","reported"));
        assert!(runtime.last_result.unwrap()["risks"][0].as_str().unwrap().contains("partial"));
        let entries = crate::store::session_entries(&db, "child").unwrap();
        assert!(entries.iter().any(|entry| entry.payload["reason"] == "supervisor_restart_orphan"));
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE entity_id='child' AND kind='adapter.orphan_killed')", [], |row| row.get::<_,bool>(0)).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn restart_refuses_to_kill_pid_when_identity_does_not_match() {
        let db = database();
        let mut process = sleeping_process();
        let pid = process.id();
        db.execute("UPDATE sessions SET adapter_pid=?2,adapter_process_identity='different process',active_turn_id='turn' WHERE id='parent'", params!["parent",i64::from(pid)]).unwrap();
        assert_eq!(SessionSupervisor::recover_tracked_adapter_processes(&db).unwrap(), 1);
        assert!(process.try_wait().unwrap().is_none());
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE entity_id='parent' AND kind='adapter.orphan_identity_mismatch')", [], |row| row.get::<_,bool>(0)).unwrap());
        assert_eq!(db.query_row("SELECT status FROM sessions WHERE id='parent'", [], |row| row.get::<_,String>(0)).unwrap(), "failed");
        db.execute("UPDATE sessions SET status='stopped' WHERE id='child'", []).unwrap();
        SessionSupervisor::reconcile_workspace_statuses(&db).unwrap();
        assert_eq!(db.query_row("SELECT status FROM workspaces WHERE id='w'", [], |row| row.get::<_,String>(0)).unwrap(), "failed");
        let _ = adapters::terminate_process_group(pid);
        let _ = process.wait();
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
        let reported = SessionSupervisor::record_result(&db, "child", &result).unwrap().unwrap();
        assert_eq!(reported.parent_session_id, "parent");
        assert!(!reported.evidence_id.is_empty());
        let evidence = SessionSupervisor::worker_evidence(&db, "parent", &[]).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_id, reported.evidence_id);
        assert_eq!(evidence[0].child_session_id, "child");
        assert_eq!(evidence[0].result, result);
        let child_result_id = store::session_entries(&db, "child").unwrap().last().unwrap().id.clone();
        assert!(SessionSupervisor::worker_evidence(&db, "parent", &[child_result_id]).is_err());
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
    fn evidence_selection_is_active_branch_only_and_malformed_records_fail_closed() {
        let db = database();
        let result = WorkerResult {
            schema_version: 1,
            status: WorkerResultStatus::Completed,
            summary: "Canonical result".into(),
            files_changed: vec!["src/lib.rs".into()], tests: vec![], decisions: vec!["Keep the API stable".into()], risks: vec![], remaining_work: vec![],
            suggested_next_action: SuggestedNextAction::Finish,
            suggested_role: None, suggested_task: None,
        };
        let reported = SessionSupervisor::record_result(&db, "child", &result).unwrap().unwrap();
        let forest = session_forest::SessionForest::new(&db);
        let mut payload = serde_json::to_value(&result).unwrap();
        payload["childSessionId"] = serde_json::json!("abandoned-child");
        let abandoned = forest.append("parent", EntryKind::WorkerResult, payload).unwrap();
        let selected = SessionSupervisor::worker_evidence(&db, "parent", &[abandoned.id.clone(), reported.evidence_id.clone()]).unwrap();
        assert_eq!(selected.iter().map(|item| item.evidence_id.as_str()).collect::<Vec<_>>(), [abandoned.id.as_str(), reported.evidence_id.as_str()]);
        assert!(SessionSupervisor::worker_evidence(&db, "parent", &[reported.evidence_id.clone(), reported.evidence_id.clone()]).is_err());
        forest.move_head("parent", Some(&reported.evidence_id)).unwrap();
        let non_result = forest.append("parent", EntryKind::UserMessage, serde_json::json!({"text":"continue"})).unwrap();
        let default = SessionSupervisor::worker_evidence(&db, "parent", &[]).unwrap();
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].evidence_id, reported.evidence_id);
        assert!(SessionSupervisor::worker_evidence(&db, "parent", &[abandoned.id]).is_err());
        assert!(SessionSupervisor::worker_evidence(&db, "parent", &[non_result.id]).is_err());
        let malformed = serde_json::json!({"status":"completed","childSessionId":"child"}).to_string();
        db.execute("UPDATE session_entries SET payload=?2 WHERE id=?1", params![reported.evidence_id, malformed]).unwrap();
        assert!(SessionSupervisor::worker_evidence(&db, "parent", &[]).is_err());
    }

    #[test]
    fn default_evidence_is_deterministic_and_bounded_to_recent_results() {
        let db = database();
        let forest = session_forest::SessionForest::new(&db);
        for index in 0..=MAX_EVIDENCE_REFERENCES {
            let result = WorkerResult {
                schema_version: 1, status: WorkerResultStatus::Completed,
                summary: format!("result-{index}"), files_changed: vec![], tests: vec![], decisions: vec![], risks: vec![], remaining_work: vec![],
                suggested_next_action: SuggestedNextAction::Finish, suggested_role: None, suggested_task: None,
            };
            let mut payload = serde_json::to_value(result).unwrap();
            payload["childSessionId"] = serde_json::json!(format!("child-{index}"));
            forest.append("parent", EntryKind::WorkerResult, payload).unwrap();
        }
        let evidence = SessionSupervisor::worker_evidence(&db, "parent", &[]).unwrap();
        assert_eq!(evidence.len(), MAX_EVIDENCE_REFERENCES);
        assert_eq!(evidence.first().unwrap().result.summary, "result-1");
        assert_eq!(evidence.last().unwrap().result.summary, format!("result-{MAX_EVIDENCE_REFERENCES}"));
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
        SessionSupervisor::reconcile_workspace_statuses(&db).unwrap();
        assert_eq!(db.query_row("SELECT status FROM workspaces WHERE id='w'", [], |row| row.get::<_,String>(0)).unwrap(), "ready");
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
        let reported = SessionSupervisor::record_result(&db, "child", &result).unwrap().unwrap();
        assert_eq!(reported.parent_session_id, "parent");
        assert!(!reported.evidence_id.is_empty());
        let runtime = store::worker_runtime(&db, "child").unwrap().unwrap();
        assert_eq!((runtime.lifecycle_state.as_str(), runtime.result_status.as_str(), runtime.retry_count), ("cancelled", "reported", 0));
        assert_eq!(runtime.last_result.unwrap()["status"], "cancelled");
        let lease: String = db.query_row("SELECT lease_status FROM worker_leases WHERE session_id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(lease, "released");
        assert_eq!(store::outstanding_children(&db, "parent").unwrap(), 0);
    }
}
