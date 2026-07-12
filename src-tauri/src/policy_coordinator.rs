use crate::{delegation::DelegationRequest, policy, BridgeError};
use rusqlite::{params, Connection};

pub struct WorkerRouteContext {
    pub workspace_id: String,
    pub parent_depth: i64,
    pub path: String,
    pub branch: String,
    pub outcome: policy::PolicyOutcome,
}

pub struct PolicyCoordinator;

impl PolicyCoordinator {
    pub fn decide_worker_route(
        db: &Connection,
        parent_session_id: &str,
        turn_id: &str,
        request: &DelegationRequest,
        child_worktrees_available: bool,
    ) -> Result<WorkerRouteContext, BridgeError> {
        let (workspace_id, parent_depth, path, branch): (String, i64, String, String) = db
            .query_row(
                "SELECT s.workspace_id,COALESCE(s.depth,0),w.path,w.branch FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
                params![parent_session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let budget = policy::load_request_budget(db, &workspace_id, turn_id)?;
        let input = policy::PolicyInput {
            workspace_id: workspace_id.clone(),
            worktree_id: workspace_id.clone(),
            parent_session_id: parent_session_id.into(),
            turn_id: turn_id.into(),
            parent_depth,
            request: request.clone(),
            requested_harness: request.runtime_harness(),
            task_family: policy::role_name(request.role).into(),
            active_workers: policy::load_workers(db, &workspace_id, "active")?,
            warm_workers: policy::load_workers(db, &workspace_id, "warm")?,
            budget: budget.clone(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available,
        };
        let outcome = policy::PolicyEngine::default().decide(&input);
        policy::record_decision(
            db,
            parent_session_id,
            turn_id,
            request,
            &outcome,
            &budget,
        )?;
        Ok(WorkerRouteContext {
            workspace_id,
            parent_depth,
            path,
            branch,
            outcome,
        })
    }
}
