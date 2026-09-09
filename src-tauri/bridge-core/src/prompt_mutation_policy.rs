//! Host-derived authorization for proposing changes to shared role guidance.
//! Provider permission convenience settings and routing scores are not inputs.

use crate::{agent_config, delegation::WorkerRole, prompts::PromptTarget, BridgeError};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMutationAuthority {
    pub actor_role: String,
    pub target: PromptTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptMutationDecision {
    RequireApproval(PromptMutationAuthority),
    Reject(String),
}

struct SessionAuthority {
    kind: String,
    status: String,
    active_turn_id: Option<String>,
    parent: Option<String>,
    workspace: Option<String>,
    depth: i64,
    worker_parent: Option<String>,
    worker_role: Option<String>,
    worker_state: Option<String>,
    worker_result: Option<String>,
}

fn session(db: &Connection, id: &str) -> Result<Option<SessionAuthority>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT s.kind,s.status,s.active_turn_id,s.parent_session_id,s.workspace_id,
                COALESCE(s.depth,0),r.parent_session_id,l.role,r.lifecycle_state,r.result_status
         FROM sessions s LEFT JOIN worker_runtime r ON r.session_id=s.id
         LEFT JOIN worker_leases l ON l.session_id=s.id WHERE s.id=?1",
            params![id],
            |row| {
                Ok(SessionAuthority {
                    kind: row.get(0)?,
                    status: row.get(1)?,
                    active_turn_id: row.get(2)?,
                    parent: row.get(3)?,
                    workspace: row.get(4)?,
                    depth: row.get(5)?,
                    worker_parent: row.get(6)?,
                    worker_role: row.get(7)?,
                    worker_state: row.get(8)?,
                    worker_result: row.get(9)?,
                })
            },
        )
        .optional()?)
}

impl SessionAuthority {
    fn orchestrator(&self) -> bool {
        self.kind == "orchestrator"
            && self.depth == 0
            && self.parent.is_none()
            && self.worker_parent.is_none()
    }

    fn worker(&self) -> Option<WorkerRole> {
        // Old reservations used the default orchestrator kind; a worker runtime
        // and matching parent/depth, never the label, establish worker identity.
        if !matches!(self.kind.as_str(), "worker" | "workspace" | "orchestrator")
            || self.depth != 1
            || self.parent.is_none()
            || self.parent != self.worker_parent
        {
            return None;
        }
        let role = self.worker_role.as_deref()?;
        WorkerRole::parse(role).filter(|parsed| parsed.as_str() == role)
    }
}

/// Revalidation permits an already-proposed actor to finish; a new proposal
/// always needs a live, host-bound turn. Both paths recheck grants/ownership.
pub fn decide(
    db: &Connection,
    actor_session_id: &str,
    actor_turn_id: &str,
    target_session_id: &str,
    reviewing_existing: bool,
) -> Result<PromptMutationDecision, BridgeError> {
    let reject = |reason: &str| Ok(PromptMutationDecision::Reject(reason.into()));
    let Some(actor) = session(db, actor_session_id)? else {
        return reject("Prompt changes require an existing actor session");
    };
    if !reviewing_existing
        && (actor.active_turn_id.as_deref() != Some(actor_turn_id)
            || !matches!(actor.status.as_str(), "working" | "waiting")
            || actor
                .worker_state
                .as_deref()
                .is_some_and(|state| !matches!(state, "working" | "waiting"))
            || actor
                .worker_result
                .as_deref()
                .is_some_and(|status| status != "pending"))
    {
        return reject("Prompt changes require the actor's current live assistant turn");
    }
    let target = if actor.orchestrator() {
        if actor_session_id == target_session_id {
            PromptTarget::Orchestrator
        } else {
            let Some(target) = session(db, target_session_id)? else {
                return reject("Prompt change target session does not exist");
            };
            let Some(role) = target.worker() else {
                return reject("An orchestrator may target only its own worker sessions");
            };
            if target.parent.as_deref() != Some(actor_session_id)
                || target.workspace != actor.workspace
            {
                return reject("The target worker does not belong to this orchestrator");
            }
            PromptTarget::Worker(role)
        }
    } else if let Some(role) = actor.worker() {
        if actor_session_id != target_session_id {
            return reject("A worker may propose only for its own shared role");
        }
        let Some(parent) = session(db, actor.parent.as_deref().unwrap_or_default())? else {
            return reject("The worker's orchestrator no longer exists");
        };
        if !parent.orchestrator() || parent.workspace != actor.workspace {
            return reject("The worker must belong to a Bridge orchestrator");
        }
        if !agent_config::permission_policy(db)?
            .worker_prompt_proposal_roles
            .contains(&role)
        {
            return reject("Prompt proposals are not enabled for this worker role in Settings");
        }
        PromptTarget::Worker(role)
    } else {
        return reject("Direct chats and internal sessions cannot propose prompt changes");
    };
    Ok(PromptMutationDecision::RequireApproval(
        PromptMutationAuthority {
            actor_role: if actor.orchestrator() {
                "orchestrator".into()
            } else {
                actor
                    .worker()
                    .expect("validated worker role")
                    .as_str()
                    .into()
            },
            target,
        },
    ))
}
