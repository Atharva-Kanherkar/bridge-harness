use crate::{
    delegation::DelegationRequest,
    model::QueuedWorkerRequest,
    policy, store, BridgeError,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const STANDARD_WARM_TIMEOUT_MINUTES: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCompatibilityKey {
    pub workspace_id: String,
    pub role: String,
    pub harness: String,
    pub capability_tier: String,
    pub task_family: String,
    pub owned_paths: Vec<String>,
}

impl WorkerCompatibilityKey {
    pub fn for_request(
        workspace_id: &str,
        request: &DelegationRequest,
    ) -> Result<Self, BridgeError> {
        Ok(Self {
            workspace_id: workspace_id.to_owned(),
            role: policy::role_name(request.role).to_owned(),
            harness: request.runtime_harness().to_owned(),
            capability_tier: request.capability_tier.as_str().to_owned(),
            task_family: task_family(request),
            owned_paths: policy::normalize_owned_paths(&request.owned_paths)
                .map_err(BridgeError::Invalid)?,
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
    let reusable_implementation = role == "implementation"
        && capability_tier == "standard"
        && write_mode != "read_only";
    if reusable_implementation {
        RetentionAction::KeepWarmUntil(now + Duration::minutes(STANDARD_WARM_TIMEOUT_MINUTES))
    } else {
        RetentionAction::StopImmediately
    }
}

pub struct WorkerPool;

impl WorkerPool {
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
                created_at: now.clone(),
                updated_at: now,
            },
        )?;
        Ok(id)
    }
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
            relevant_files: vec!["src/auth.rs".into()],
            owned_paths: vec!["src/auth/**".into(), "src/auth.rs".into()],
            write_mode: WriteMode::Shared,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            verification: vec!["cargo test auth".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: None,
            model: None,
        }
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
        let encoded = key.encode().unwrap();
        let mut changed = key.clone();
        changed.harness = "claude".into();
        assert_ne!(encoded, changed.encode().unwrap());
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
            assert_eq!(retention_action(&one_shot, now), RetentionAction::StopImmediately);
        }
    }
}
