use crate::{
    compaction_controller::CompactionController, delegation::DelegationRequest, handoff,
    model::QueuedWorkerRequest, policy, session_supervisor::SessionSupervisor, store,
    worker_lifecycle::WorkerLifecycleState, BridgeError,
};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STANDARD_WARM_TIMEOUT_MINUTES: i64 = 5;
pub const QUEUE_TTL_HOURS: i64 = 24;
pub const DISPATCH_LEASE_MINUTES: i64 = 5;
pub const MAX_QUEUE_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCompatibilityKey {
    pub workspace_id: String,
    pub role: String,
    pub harness: String,
    pub capability_tier: String,
    pub task_family: String,
    pub owned_paths: Vec<String>,
    pub write_mode: String,
    pub network_access: bool,
    pub writable_output_paths: Vec<String>,
}

impl WorkerCompatibilityKey {
    pub fn for_request(
        workspace_id: &str,
        request: &DelegationRequest,
    ) -> Result<Self, BridgeError> {
        let mut writable_output_paths = request.writable_output_paths.clone();
        writable_output_paths.sort();
        Ok(Self {
            workspace_id: workspace_id.to_owned(),
            role: policy::role_name(request.role).to_owned(),
            harness: request.runtime_harness().to_owned(),
            capability_tier: request.capability_tier.as_str().to_owned(),
            task_family: task_family(request),
            owned_paths: policy::normalize_owned_paths(&request.owned_paths)
                .map_err(BridgeError::Invalid)?,
            write_mode: policy::write_mode_name(request.write_mode).to_owned(),
            network_access: request.network_access,
            writable_output_paths,
        })
    }

    pub fn encode(&self) -> Result<String, BridgeError> {
        serde_json::to_string(self).map_err(|error| BridgeError::Invalid(error.to_string()))
    }
}

pub fn task_family(request: &DelegationRequest) -> String {
    policy::role_name(request.role).to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionAction {
    StopImmediately,
    KeepWarmUntil(DateTime<Utc>),
}

pub fn retention_action(request: &DelegationRequest, now: DateTime<Utc>) -> RetentionAction {
    retention_action_for_attributes(
        policy::role_name(request.role),
        request.capability_tier.as_str(),
        policy::write_mode_name(request.write_mode),
        now,
    )
}

pub fn retention_action_for_attributes(
    role: &str,
    capability_tier: &str,
    write_mode: &str,
    now: DateTime<Utc>,
) -> RetentionAction {
    let reusable_implementation =
        role == "implementation" && capability_tier == "standard" && write_mode != "readOnly";
    if reusable_implementation {
        RetentionAction::KeepWarmUntil(now + Duration::minutes(STANDARD_WARM_TIMEOUT_MINUTES))
    } else {
        RetentionAction::StopImmediately
    }
}

pub struct WorkerPool;

impl WorkerPool {
    pub fn warm_workers_due(
        db: &Connection,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, BridgeError> {
        let mut statement = db.prepare(
            "SELECT session_id FROM worker_runtime
             WHERE lifecycle_state='warm' AND warm_until IS NOT NULL AND warm_until<=?1
             ORDER BY warm_until,session_id",
        )?;
        let rows = statement
            .query_map([now.to_rfc3339()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn enqueue(
        db: &Connection,
        parent_session_id: &str,
        workspace_id: &str,
        turn_id: &str,
        request: &DelegationRequest,
        actual_model: &str,
    ) -> Result<String, BridgeError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::hours(QUEUE_TTL_HOURS)).to_rfc3339();
        store::enqueue_worker_request(
            db,
            &QueuedWorkerRequest {
                id: id.clone(),
                parent_session_id: parent_session_id.to_owned(),
                workspace_id: workspace_id.to_owned(),
                turn_id: turn_id.to_owned(),
                request: serde_json::to_value(request)
                    .map_err(|error| BridgeError::Invalid(error.to_string()))?,
                actual_model: actual_model.to_owned(),
                queue_status: "queued".into(),
                sequence: 0,
                dispatched_session_id: None,
                attempt_count: 0,
                expires_at,
                blocked_at: None,
                claimed_at: None,
                last_error: None,
                created_at: now.clone(),
                updated_at: now,
            },
        )?;
        Ok(id)
    }

    pub fn maintain_queue(db: &Connection, now: DateTime<Utc>) -> Result<(), BridgeError> {
        let stale_before = (now - Duration::minutes(DISPATCH_LEASE_MINUTES)).to_rfc3339();
        let now_text = now.to_rfc3339();
        db.execute("UPDATE worker_queue SET queue_status='cancelled',blocked_at=NULL,last_error='parent session cancelled',updated_at=?1 WHERE queue_status IN ('queued','dispatching','blocked_on_human') AND parent_session_id IN (SELECT id FROM sessions WHERE status='cancelled')", rusqlite::params![now_text])?;
        db.execute("UPDATE worker_queue SET queue_status=CASE WHEN attempt_count>=?1 THEN 'dead_letter' ELSE 'queued' END,attempt_count=attempt_count+1,claimed_at=NULL,last_error='stale dispatch lease expired',updated_at=?2 WHERE queue_status='dispatching' AND claimed_at IS NOT NULL AND claimed_at<=?3", rusqlite::params![MAX_QUEUE_ATTEMPTS,now_text,stale_before])?;

        let pending = {
            let mut statement = db.prepare("SELECT id,parent_session_id,queue_status,expires_at,blocked_at FROM worker_queue WHERE queue_status IN ('queued','blocked_on_human') ORDER BY sequence")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for (id, parent_session_id, queue_status, expires_at, blocked_at) in pending {
            let human_blocked = Self::has_waiting_ancestor(db, &parent_session_id)?;
            if queue_status == "queued" && human_blocked {
                if db.execute("UPDATE worker_queue SET queue_status='blocked_on_human',blocked_at=?2,last_error='waiting for human approval',updated_at=?2 WHERE id=?1 AND queue_status='queued'", rusqlite::params![id,now_text])? == 1 {
                    store::event(db, "policy", "queue.blocked_on_human", &id, "Queue TTL paused while an ancestor awaits approval")?;
                }
            } else if queue_status == "blocked_on_human" && !human_blocked {
                let blocked_at = blocked_at.ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "Human-blocked queue item {id} has no blocked timestamp"
                    ))
                })?;
                let paused_for = now.signed_duration_since(parse_queue_timestamp(&blocked_at)?);
                let paused_for = if paused_for < Duration::zero() {
                    Duration::zero()
                } else {
                    paused_for
                };
                let adjusted_expiry =
                    (parse_queue_timestamp(&expires_at)? + paused_for).to_rfc3339();
                if db.execute("UPDATE worker_queue SET queue_status='queued',expires_at=?2,blocked_at=NULL,last_error=NULL,updated_at=?3 WHERE id=?1 AND queue_status='blocked_on_human'", rusqlite::params![id,adjusted_expiry,now_text])? == 1 {
                    store::event(db, "policy", "queue.released_from_human", &id, &format!("Queue TTL resumed after {} blocked seconds", paused_for.num_seconds()))?;
                }
            }
        }
        db.execute("UPDATE worker_queue SET queue_status='expired',last_error='queue TTL exceeded',updated_at=?1 WHERE queue_status='queued' AND expires_at IS NOT NULL AND expires_at<=?1", rusqlite::params![now_text])?;
        Ok(())
    }

    fn has_waiting_ancestor(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
        Ok(db.query_row(
            "WITH RECURSIVE lineage(id,parent_session_id,status) AS (
                SELECT id,parent_session_id,status FROM sessions WHERE id=?1
                UNION ALL
                SELECT s.id,s.parent_session_id,s.status FROM sessions s JOIN lineage l ON s.id=l.parent_session_id
             ) SELECT EXISTS(SELECT 1 FROM lineage WHERE status='waiting')",
            rusqlite::params![session_id],
            |row| row.get(0),
        )?)
    }

    pub fn expire_warm_workers(
        db: &Connection,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, BridgeError> {
        let session_ids = Self::warm_workers_due(db, now)?;
        for session_id in &session_ids {
            CompactionController::request_before_suspend(db, session_id, "warm_idle_timeout")?;
            SessionSupervisor::transition(
                db,
                session_id,
                WorkerLifecycleState::Checkpointing,
                Some("warm_idle_timeout"),
            )?;
            SessionSupervisor::transition(
                db,
                session_id,
                WorkerLifecycleState::Stopped,
                Some("checkpoint_requested"),
            )?;
            db.execute(
                "UPDATE worker_leases SET lease_status='checkpointed',updated_at=?2 WHERE session_id=?1",
                rusqlite::params![session_id, now.to_rfc3339()],
            )?;
        }
        Ok(session_ids)
    }

    pub fn claim_next_queued(
        db: &Connection,
        workspace_id: &str,
    ) -> Result<Option<QueuedWorkerRequest>, BridgeError> {
        Self::maintain_queue(db, Utc::now())?;
        let active = policy::load_workers(db, workspace_id, "active")?;
        if active.len() >= policy::PolicyConfig::default().max_concurrent_workers {
            return Ok(None);
        }
        let Some(request) = store::queued_worker_requests(db, workspace_id)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let directive: DelegationRequest = match serde_json::from_value::<DelegationRequest>(
            request.request.clone(),
        ) {
            Ok(directive) if directive.validate().is_ok() => directive,
            _ => {
                db.execute("UPDATE worker_queue SET queue_status='dead_letter',last_error='invalid queued delegation request',updated_at=?2 WHERE id=?1", rusqlite::params![request.id,Utc::now().to_rfc3339()])?;
                return Ok(None);
            }
        };
        let handoff =
            handoff::assess(db, &request.parent_session_id, &directive.runtime_harness())?;
        if handoff.cross_harness && !handoff.at_phase_boundary {
            return Ok(None);
        }
        let conflicts = directive.write_mode != crate::delegation::WriteMode::ReadOnly
            && active.iter().any(|worker| {
                worker.write_mode != crate::delegation::WriteMode::ReadOnly
                    && policy::owned_path_sets_overlap(&directive.owned_paths, &worker.owned_paths)
                        .unwrap_or(true)
            });
        if conflicts {
            return Ok(None);
        }
        let claimed = db.execute(
            "UPDATE worker_queue SET queue_status='dispatching',attempt_count=attempt_count+1,claimed_at=?2,updated_at=?2 WHERE id=?1 AND queue_status='queued'",
            rusqlite::params![request.id, Utc::now().to_rfc3339()],
        )? == 1;
        Ok(claimed.then_some(request))
    }

    pub fn activate_reused_worker(
        db: &Connection,
        session_id: &str,
        workspace_id: &str,
        parent_session_id: &str,
        depth: i64,
        request: &DelegationRequest,
    ) -> Result<(), BridgeError> {
        let key = WorkerCompatibilityKey::for_request(workspace_id, request)?.encode()?;
        let now = Utc::now().to_rfc3339();
        // A reused worker answers to the orchestrator that resumed it. Leaving
        // the previous parent in place hides the worker from the new session's
        // tree and routes its result to a parent that may no longer exist.
        db.execute(
            "UPDATE worker_runtime SET result_status='pending',warm_until=NULL,compatibility_key=?2,parent_session_id=?4,updated_at=?3 WHERE session_id=?1",
            rusqlite::params![session_id, key, now, parent_session_id],
        )?;
        db.execute(
            "UPDATE sessions SET parent_session_id=?2,depth=?3,ended_at=NULL WHERE id=?1",
            rusqlite::params![session_id, parent_session_id, depth],
        )?;
        db.execute(
            "UPDATE worker_leases SET role=?2,capability_tier=?3,task_family=?4,owned_paths=?5,write_mode=?6,lease_status='active',expires_at=NULL,updated_at=?7 WHERE session_id=?1",
            rusqlite::params![session_id,policy::role_name(request.role),request.capability_tier.as_str(),task_family(request),serde_json::to_string(&request.owned_paths).unwrap_or_else(|_| "[]".into()),policy::write_mode_name(request.write_mode),now],
        )?;
        Ok(())
    }
}

fn parse_queue_timestamp(value: &str) -> Result<DateTime<Utc>, BridgeError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|timestamp| timestamp.and_utc())
        })
        .map_err(|_| BridgeError::Invalid(format!("Invalid queue timestamp: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::{Effort, OutputContract, WorkerRole, WriteMode},
        model::CapabilityTier,
    };

    fn request() -> DelegationRequest {
        DelegationRequest {
            schema_version: 1,
            role: WorkerRole::Implementation,
            objective: "Implement auth".into(),
            acceptance_criteria: vec!["tests pass".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec!["src/auth.rs".into()],
            owned_paths: vec!["src/auth/**".into(), "src/auth.rs".into()],
            write_mode: WriteMode::Shared,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            network_access: false,
            writable_output_paths: vec![],
            verification: vec!["cargo test auth".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: None,
            model: None,
        }
    }

    #[test]
    fn activating_a_reused_worker_reparents_it_to_the_resuming_orchestrator() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        for id in ["old-parent", "new-parent"] {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES(?1,NULL,'claude','Orchestrator','ready','reported')",
                rusqlite::params![id],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth,ended_at) VALUES('worker-1',NULL,'claude','Research · standard','restored','reported','old-parent',1,'2026-08-19T00:00:00Z')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at) VALUES('worker-1','old-parent','restored','research','stale-key','reported','2026-08-18T00:00:00Z')",
            [],
        )
        .unwrap();

        WorkerPool::activate_reused_worker(&db, "worker-1", "workspace-1", "new-parent", 1, &request())
            .unwrap();

        let (runtime_parent, result_status): (String, String) = db
            .query_row(
                "SELECT parent_session_id,result_status FROM worker_runtime WHERE session_id='worker-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(runtime_parent, "new-parent", "results must route to the resuming parent");
        assert_eq!(result_status, "pending", "a stale reported status would swallow the new result");
        let (session_parent, depth, ended_at): (String, i64, Option<String>) = db
            .query_row(
                "SELECT parent_session_id,depth,ended_at FROM sessions WHERE id='worker-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(session_parent, "new-parent", "the tree must show the worker under the new parent");
        assert_eq!(depth, 1);
        assert_eq!(ended_at, None, "a resumed worker is not ended");
    }

    #[test]
    fn compatibility_key_uses_every_required_dimension_and_normalizes_paths() {
        let key = WorkerCompatibilityKey::for_request("workspace", &request()).unwrap();
        assert_eq!(key.workspace_id, "workspace");
        assert_eq!(key.role, "implementation");
        assert_eq!(key.harness, "codex");
        assert_eq!(key.capability_tier, "standard");
        assert_eq!(key.task_family, "implementation");
        assert_eq!(key.owned_paths, vec!["src/auth.rs", "src/auth/**"]);
        assert_eq!(key.write_mode, "shared");
        assert!(!key.network_access);
        assert!(key.writable_output_paths.is_empty());
        let encoded = key.encode().unwrap();
        let mut changed = key.clone();
        changed.harness = "claude".into();
        assert_ne!(encoded, changed.encode().unwrap());

        let mut read_only = request();
        read_only.write_mode = WriteMode::ReadOnly;
        assert_ne!(
            encoded,
            WorkerCompatibilityKey::for_request("workspace", &read_only)
                .unwrap()
                .encode()
                .unwrap()
        );
    }

    #[test]
    fn only_standard_writing_implementation_workers_are_kept_warm() {
        let now = DateTime::parse_from_rfc3339("2026-07-13T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let standard = request();
        assert_eq!(
            retention_action(&standard, now),
            RetentionAction::KeepWarmUntil(now + Duration::minutes(5))
        );
        let mutations: [fn(&mut DelegationRequest); 4] = [
            |request: &mut DelegationRequest| request.write_mode = WriteMode::ReadOnly,
            |request: &mut DelegationRequest| request.capability_tier = CapabilityTier::Fast,
            |request: &mut DelegationRequest| request.capability_tier = CapabilityTier::Strong,
            |request: &mut DelegationRequest| request.role = WorkerRole::Verification,
        ];
        for mutate in mutations {
            let mut one_shot = request();
            mutate(&mut one_shot);
            assert_eq!(
                retention_action(&one_shot, now),
                RetentionAction::StopImmediately
            );
        }
    }

    #[test]
    fn cancellation_is_terminal_and_never_retried() {
        let result = crate::delegation::WorkerResult {
            schema_version: crate::delegation::SCHEMA_VERSION,
            status: crate::delegation::WorkerResultStatus::Cancelled,
            summary: "cancelled".into(),
            files_changed: vec![],
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: crate::delegation::SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        assert!(result.is_terminal_cancellation());
        // Retry policy lives in `crate::worker_retry` now: this used to retry
        // any typed failure once, without asking whether the cause could have
        // changed. Keeping a second, laxer answer around was an invitation.
        assert!(!crate::worker_retry::decide(&result, 0, true, 0).is_retry());
    }

    #[test]
    fn expired_warm_worker_requests_checkpoint_then_stops() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/pool','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/pool-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','codex','Worker','warm','reported','parent',1)", []).unwrap();
        store::upsert_worker_runtime(
            &db,
            &crate::model::WorkerRuntimeRecord {
                session_id: "child".into(),
                parent_session_id: "parent".into(),
                lifecycle_state: "warm".into(),
                task_family: "implementation".into(),
                compatibility_key: "key".into(),
                result_status: "reported".into(),
                retry_count: 0,
                warm_until: Some("2026-07-13T00:00:00+00:00".into()),
                worktree_path: None,
                worktree_branch: None,
                last_result: None,
                last_activity_at: None,
                waiting_since: None,
                waiting_reason: None,
                progress_summary: None,
                updated_at: "now".into(),
            },
        )
        .unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,write_mode,lease_status,created_at,updated_at) VALUES('child','w','implementation','standard','implementation','shared','warm','now','now')", []).unwrap();

        let now = DateTime::parse_from_rfc3339("2026-07-13T00:01:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            WorkerPool::expire_warm_workers(&db, now).unwrap(),
            vec!["child"]
        );
        assert_eq!(
            store::worker_runtime(&db, "child")
                .unwrap()
                .unwrap()
                .lifecycle_state,
            "stopped"
        );
        let entries = store::session_entries(&db, "child").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["compaction.requested", "session.status", "session.status"]
        );
        assert_eq!(entries[1].payload["status"], "checkpointing");
        assert_eq!(entries[2].payload["status"], "stopped");
    }

    #[test]
    fn fifo_queue_waits_for_conflicting_writer_then_claims_oldest() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/queue','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/queue-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('active','w','codex','Worker','working','reported','parent',1)", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at) VALUES('active','w','implementation','standard','implementation','[\"src/**\"]','shared','active','now','now')", []).unwrap();
        let directive = request();
        for id in ["q1", "q2"] {
            store::enqueue_worker_request(
                &db,
                &QueuedWorkerRequest {
                    id: id.into(),
                    parent_session_id: "parent".into(),
                    workspace_id: "w".into(),
                    turn_id: "turn".into(),
                    request: serde_json::to_value(&directive).unwrap(),
                    actual_model: "model".into(),
                    queue_status: "queued".into(),
                    sequence: 0,
                    dispatched_session_id: None,
                    attempt_count: 0,
                    expires_at: "2099-01-01T00:00:00+00:00".into(),
                    blocked_at: None,
                    claimed_at: None,
                    last_error: None,
                    created_at: "now".into(),
                    updated_at: "now".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(WorkerPool::claim_next_queued(&db, "w").unwrap(), None);
        db.execute(
            "UPDATE worker_leases SET lease_status='released' WHERE session_id='active'",
            [],
        )
        .unwrap();
        assert_eq!(
            WorkerPool::claim_next_queued(&db, "w").unwrap().unwrap().id,
            "q1"
        );
        store::update_worker_queue(&db, "q1", "dispatched", Some("active")).unwrap();
        assert_eq!(
            WorkerPool::claim_next_queued(&db, "w").unwrap().unwrap().id,
            "q2"
        );
    }

    #[test]
    fn cross_harness_queue_waits_for_parent_phase_boundary() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/handoff-queue','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/handoff-queue-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,active_turn_id) VALUES('parent','w','codex','Parent','working','reported','turn')", []).unwrap();
        let mut directive = request();
        directive.harness = Some("claude".into());
        WorkerPool::enqueue(&db, "parent", "w", "turn", &directive, "model").unwrap();
        assert_eq!(WorkerPool::claim_next_queued(&db, "w").unwrap(), None);
        db.execute(
            "UPDATE sessions SET active_turn_id=NULL WHERE id='parent'",
            [],
        )
        .unwrap();
        assert!(WorkerPool::claim_next_queued(&db, "w").unwrap().is_some());
    }

    #[test]
    fn human_approval_pauses_transitive_queue_ttl_and_releases_with_time_restored() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/human-queue','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/human-queue-w','waiting','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('root','w','codex','Root','waiting','reported')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('child','w','codex','Child','working','reported','root')", []).unwrap();
        let directive = request();
        store::enqueue_worker_request(
            &db,
            &QueuedWorkerRequest {
                id: "q-human".into(),
                parent_session_id: "child".into(),
                workspace_id: "w".into(),
                turn_id: "turn".into(),
                request: serde_json::to_value(&directive).unwrap(),
                actual_model: "model".into(),
                queue_status: "queued".into(),
                sequence: 0,
                dispatched_session_id: None,
                attempt_count: 0,
                expires_at: "2026-07-14T00:10:00+00:00".into(),
                blocked_at: None,
                claimed_at: None,
                last_error: None,
                created_at: "2026-07-13T00:00:00+00:00".into(),
                updated_at: "2026-07-13T00:00:00+00:00".into(),
            },
        )
        .unwrap();

        let blocked_at = DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        WorkerPool::maintain_queue(&db, blocked_at).unwrap();
        let blocked = store::worker_queue_requests(&db, "w").unwrap().remove(0);
        assert_eq!(blocked.queue_status, "blocked_on_human");
        assert_eq!(blocked.expires_at, "2026-07-14T00:10:00+00:00");

        db.execute("UPDATE sessions SET status='working' WHERE id='root'", [])
            .unwrap();
        db.execute("UPDATE sessions SET status='waiting' WHERE id='child'", [])
            .unwrap();
        let directly_blocked_at = DateTime::parse_from_rfc3339("2026-07-14T00:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        WorkerPool::maintain_queue(&db, directly_blocked_at).unwrap();
        assert_eq!(
            store::worker_queue_requests(&db, "w").unwrap()[0].queue_status,
            "blocked_on_human"
        );
        db.execute("UPDATE sessions SET status='working' WHERE id='child'", [])
            .unwrap();
        let released_at = DateTime::parse_from_rfc3339("2026-07-14T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        WorkerPool::maintain_queue(&db, released_at).unwrap();
        let released = store::worker_queue_requests(&db, "w").unwrap().remove(0);
        assert_eq!(released.queue_status, "queued");
        assert_eq!(released.expires_at, "2026-07-14T01:10:00+00:00");
        assert_eq!(released.blocked_at, None);
        let events = store::workspace_reason_events(&db, "w").unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == "queue.blocked_on_human"));
        assert!(events
            .iter()
            .any(|event| event.kind == "queue.released_from_human"));
    }

    #[test]
    fn cancellation_terminates_human_blocked_queue_items() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/cancel-human','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/cancel-human-w','waiting','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','cancelled','reported')", []).unwrap();
        db.execute("INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,expires_at,blocked_at,created_at,updated_at) VALUES('q','parent','w','turn','{}','model','blocked_on_human','2099-01-01T00:00:00+00:00','2026-07-14T00:00:00+00:00','now','now')", []).unwrap();
        WorkerPool::maintain_queue(&db, Utc::now()).unwrap();
        assert_eq!(
            store::worker_queue_requests(&db, "w").unwrap()[0].queue_status,
            "cancelled"
        );
    }
}
