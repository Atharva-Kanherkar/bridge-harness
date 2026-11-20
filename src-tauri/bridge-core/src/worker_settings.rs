//! Workspace-local operational limits. Write-scope and independent verification
//! remain policy invariants, not settings a user can accidentally disable.
use crate::{policy::PolicyConfig, BridgeError};
use bridge_protocol::messages::WorkerSettings;
use rusqlite::{params, Connection, OptionalExtension};

pub fn load(db: &Connection, workspace: &str) -> Result<WorkerSettings, BridgeError> {
    let payload: Option<String> = db.query_row(
        "SELECT payload FROM configuration_entries WHERE kind='worker_policy' AND id=?1",
        params![workspace], |row| row.get(0),
    ).optional()?;
    payload.map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .unwrap_or_else(|| Ok(WorkerSettings::default()))
}

pub fn for_session(db: &Connection, session: &str) -> WorkerSettings {
    let workspace: Option<String> = db.query_row("SELECT workspace_id FROM sessions WHERE id=?1", params![session], |row| row.get(0)).ok().flatten();
    workspace.and_then(|id| load(db, &id).ok()).unwrap_or_default()
}

pub fn policy(db: &Connection, workspace: &str) -> Result<PolicyConfig, BridgeError> {
    let settings = load(db, workspace)?;
    Ok(PolicyConfig { max_concurrent_workers: limit(settings.max_concurrent_workers),
        max_workers_per_turn: limit(settings.max_workers_per_turn),
        max_automatic_retries: usize::from(settings.automatic_retry), ..PolicyConfig::default() })
}

/// A stored limit of `0` is "no limit", the default: workers are separate
/// processes in their own sandboxes or worktrees, so nothing about correctness
/// needs a count cap. Writer conflicts and write scope are policy invariants
/// enforced elsewhere, not by this number.
pub const fn limit(stored: usize) -> usize {
    if stored == 0 { usize::MAX } else { stored }
}

pub fn save(db: &Connection, workspace: &str, settings: &WorkerSettings) -> Result<WorkerSettings, BridgeError> {
    if settings.max_concurrent_workers > 64
        || settings.max_workers_per_turn > 64
        || !(60..=7200).contains(&settings.stall_timeout_seconds)
        || !(0..=60).contains(&settings.warm_retention_minutes)
        || settings.default_harness.as_deref().is_some_and(|id| !matches!(id, "codex" | "claude" | "opencode")) {
        return Err(BridgeError::Invalid("Invalid worker limits: concurrency/turn 0-64 (0 is no limit), stall 60-7200 seconds, warm retention 0-60 minutes, or unknown harness".into()));
    }
    db.query_row("SELECT id FROM workspaces WHERE id=?1", params![workspace], |_| Ok(()))?;
    db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
        VALUES('worker_policy',?1,?2,?3,?3) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![workspace, serde_json::to_string(settings).map_err(|error| BridgeError::Invalid(error.to_string()))?, chrono::Utc::now().to_rfc3339()])?;
    load(db, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_are_validated_persisted_and_consumed_by_policy() {
        let scratch = tempfile::tempdir().unwrap();
        let db = crate::store::open(&scratch.path().join("test.db")).unwrap();
        db.execute("INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Work','idle','2026-09-09T00:00:00Z')", []).unwrap();
        let mut settings = WorkerSettings::default();
        settings.max_concurrent_workers = 4;
        settings.automatic_retry = false;
        save(&db, "w", &settings).unwrap();
        assert_eq!(policy(&db, "w").unwrap().max_concurrent_workers, 4);
        assert_eq!(policy(&db, "w").unwrap().max_automatic_retries, 0);
        assert_eq!(load(&db, "other").unwrap().max_concurrent_workers, 0, "no limit by default");
        assert_eq!(policy(&db, "other").unwrap().max_concurrent_workers, usize::MAX);
        assert_eq!(policy(&db, "other").unwrap().max_workers_per_turn, usize::MAX);
        settings.stall_timeout_seconds = 0;
        assert!(save(&db, "w", &settings).is_err());
        assert_eq!(load(&db, "w").unwrap().stall_timeout_seconds, 600);
    }
}
