use crate::BridgeError;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub const DEFAULT_JOB_ID: &str = "default";
pub const MIN_EVIDENCE_SAMPLES: i64 = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningTriggerKind {
    Manual,
    InApp,
    Codex,
    Claude,
}

impl LearningTriggerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::InApp => "in_app",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    Noop,
}

impl LearningRunStatus {
    fn from_str(value: &str) -> Result<Self, BridgeError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "noop" => Ok(Self::Noop),
            _ => Err(BridgeError::Invalid(format!(
                "unknown learning run status {value}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Noop => "noop",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningReport {
    pub reason: String,
    pub evidence_boundary: i64,
    pub evidence_count: i64,
    pub base_policy_version: i64,
    pub candidate_policy_version: Option<i64>,
    pub quality_bps: Option<i64>,
    pub average_cost_microusd: Option<i64>,
    pub average_latency_ms: Option<i64>,
    pub policy_diff: serde_json::Value,
    pub recommendation_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningRun {
    pub id: String,
    pub job_id: String,
    pub trigger_kind: LearningTriggerKind,
    pub idempotency_key: String,
    pub evidence_boundary: i64,
    pub base_policy_version: i64,
    pub status: LearningRunStatus,
    pub report: Option<LearningReport>,
    pub candidate_policy_version: Option<i64>,
    pub cancellation_requested: bool,
    pub duplicate: bool,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningSchedule {
    pub job_id: String,
    pub enabled: bool,
    pub cadence_minutes: i64,
    pub next_run_at: Option<String>,
    pub run_budget_microusd: i64,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningState {
    pub schedule: LearningSchedule,
    pub latest_run: Option<LearningRun>,
}

fn active_policy_version(db: &Connection) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COALESCE(MAX(version),1) FROM routing_policies WHERE status='active'",
        [],
        |row| row.get(0),
    )?)
}

fn evidence_boundary(db: &Connection) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COALESCE(MAX(rowid),0) FROM router_outcomes",
        [],
        |row| row.get(0),
    )?)
}

fn record_trigger_event(
    db: &Connection,
    run_id: &str,
    trigger_kind: LearningTriggerKind,
    result: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO learning_trigger_events(id,run_id,trigger_kind,result,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![Uuid::new_v4().to_string(), run_id, trigger_kind.as_str(), result, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn run_learning(
    db: &Connection,
    trigger_kind: LearningTriggerKind,
) -> Result<LearningRun, BridgeError> {
    let boundary = evidence_boundary(db)?;
    let base_version = active_policy_version(db)?;
    let key = format!("{DEFAULT_JOB_ID}:{boundary}:{base_version}");
    if let Some(mut existing) = load_run_by_key(db, &key)? {
        record_trigger_event(db, &existing.id, trigger_kind, "duplicate_noop")?;
        existing.duplicate = true;
        return Ok(existing);
    }

    let schedule = load_schedule(db)?;
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let inserted = db.execute(
        "INSERT OR IGNORE INTO learning_job_runs(id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,'running',?7)",
        params![id, DEFAULT_JOB_ID, trigger_kind.as_str(), key, boundary, base_version, created_at],
    )?;
    if inserted == 0 {
        let mut existing = load_run_by_key(db, &key)?.ok_or_else(|| {
            BridgeError::Invalid("learning run idempotency claim was lost".into())
        })?;
        record_trigger_event(db, &existing.id, trigger_kind, "duplicate_noop")?;
        existing.duplicate = true;
        return Ok(existing);
    }
    record_trigger_event(db, &id, trigger_kind, "acquired")?;

    let (count, successes, runtime_total, cost_total, reported_costs): (i64, i64, i64, i64, i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN succeeded THEN 1 ELSE 0 END),0),COALESCE(SUM(runtime_ms),0),
                COALESCE(SUM(COALESCE((SELECT SUM(cost_microusd) FROM usage_ledger u WHERE u.session_id=router_outcomes.child_session_id),0)),0),
                COALESCE(SUM(CASE WHEN EXISTS(SELECT 1 FROM usage_ledger u WHERE u.session_id=router_outcomes.child_session_id AND cost_microusd IS NOT NULL) THEN 1 ELSE 0 END),0)
         FROM router_outcomes WHERE rowid<=?1",
        params![boundary],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;

    let mut candidate_policy_version = None;
    let (status, reason, policy_diff) = if schedule.run_budget_microusd <= 0 {
        (
            LearningRunStatus::Noop,
            "learning budget exhausted".to_owned(),
            json!({}),
        )
    } else if count < MIN_EVIDENCE_SAMPLES {
        (
            LearningRunStatus::Noop,
            format!("insufficient evidence: {count}/{MIN_EVIDENCE_SAMPLES} outcomes"),
            json!({}),
        )
    } else {
        let next_version: i64 = db.query_row(
            "SELECT COALESCE(MAX(version),0)+1 FROM routing_policies",
            [],
            |row| row.get(0),
        )?;
        let quality_bps = successes * 10_000 / count.max(1);
        let diff = json!({
            "qualityBps": quality_bps,
            "averageLatencyMs": runtime_total / count.max(1),
            "averageCostMicrousd": (reported_costs > 0).then_some(cost_total / reported_costs.max(1)),
            "guardrails": "deterministic_policy_unchanged",
        });
        db.execute(
            "INSERT INTO routing_policies(version,status,predecessor,weights,thresholds,created_reason,created_at)
             VALUES(?1,'candidate',?2,?3,?4,?5,?6)",
            params![
                next_version,
                base_version,
                json!({"observedQualityBps": quality_bps}).to_string(),
                json!({"minimumSamples": MIN_EVIDENCE_SAMPLES}).to_string(),
                format!("learning recommendation from evidence boundary {boundary}"),
                Utc::now().to_rfc3339(),
            ],
        )?;
        candidate_policy_version = Some(next_version);
        (
            LearningRunStatus::Completed,
            "candidate policy recommended".to_owned(),
            diff,
        )
    };
    let report = LearningReport {
        reason,
        evidence_boundary: boundary,
        evidence_count: count,
        base_policy_version: base_version,
        candidate_policy_version,
        quality_bps: (count > 0).then(|| successes * 10_000 / count),
        average_cost_microusd: (reported_costs > 0).then(|| cost_total / reported_costs),
        average_latency_ms: (count > 0).then(|| runtime_total / count),
        policy_diff,
        recommendation_only: true,
    };
    let completed_at = Utc::now().to_rfc3339();
    db.execute(
        "UPDATE learning_job_runs SET status=?2,report=?3,candidate_policy_version=?4,completed_at=?5 WHERE id=?1",
        params![id, status.as_str(), serde_json::to_string(&report).map_err(|error| BridgeError::Invalid(error.to_string()))?, candidate_policy_version, completed_at],
    )?;
    load_run(db, &id)?.ok_or_else(|| BridgeError::Invalid("learning run disappeared".into()))
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearningRun> {
    let trigger = match row.get::<_, String>(2)?.as_str() {
        "manual" => LearningTriggerKind::Manual,
        "in_app" => LearningTriggerKind::InApp,
        "codex" => LearningTriggerKind::Codex,
        "claude" => LearningTriggerKind::Claude,
        value => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("unknown trigger {value}").into(),
            ))
        }
    };
    let status_value = row.get::<_, String>(6)?;
    let status = LearningRunStatus::from_str(&status_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let report = row
        .get::<_, Option<String>>(7)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(LearningRun {
        id: row.get(0)?,
        job_id: row.get(1)?,
        trigger_kind: trigger,
        idempotency_key: row.get(3)?,
        evidence_boundary: row.get(4)?,
        base_policy_version: row.get(5)?,
        status,
        report,
        candidate_policy_version: row.get(8)?,
        cancellation_requested: row.get(9)?,
        duplicate: false,
        created_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

pub fn load_run(db: &Connection, id: &str) -> Result<Option<LearningRun>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,created_at,completed_at FROM learning_job_runs WHERE id=?1",
        params![id],
        map_run,
    ).optional()?)
}

fn load_run_by_key(db: &Connection, key: &str) -> Result<Option<LearningRun>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,created_at,completed_at FROM learning_job_runs WHERE idempotency_key=?1",
        params![key],
        map_run,
    ).optional()?)
}

pub fn cancel_run(db: &Connection, id: &str) -> Result<LearningRun, BridgeError> {
    db.execute(
        "UPDATE learning_job_runs SET cancellation_requested=1,status='cancelled',completed_at=?2
         WHERE id=?1 AND status IN ('queued','running') AND candidate_policy_version IS NULL",
        params![id, Utc::now().to_rfc3339()],
    )?;
    load_run(db, id)?.ok_or_else(|| BridgeError::Invalid(format!("learning run {id} not found")))
}

pub fn load_schedule(db: &Connection) -> Result<LearningSchedule, BridgeError> {
    Ok(db.query_row(
        "SELECT id,enabled,cadence_minutes,next_run_at,run_budget_microusd,mode FROM learning_jobs WHERE id=?1",
        params![DEFAULT_JOB_ID],
        |row| Ok(LearningSchedule { job_id: row.get(0)?, enabled: row.get(1)?, cadence_minutes: row.get(2)?, next_run_at: row.get(3)?, run_budget_microusd: row.get(4)?, mode: row.get(5)? }),
    )?)
}

pub fn update_schedule(
    db: &Connection,
    schedule: &LearningSchedule,
) -> Result<LearningSchedule, BridgeError> {
    if schedule.job_id != DEFAULT_JOB_ID
        || schedule.cadence_minutes < 15
        || schedule.run_budget_microusd < 0
    {
        return Err(BridgeError::Invalid("learning schedule requires the default job, cadence >= 15 minutes, and non-negative budget".into()));
    }
    if !matches!(schedule.mode.as_str(), "manual" | "ask" | "automatic") {
        return Err(BridgeError::Invalid(
            "learning mode must be manual, ask, or automatic".into(),
        ));
    }
    if let Some(next) = &schedule.next_run_at {
        DateTime::parse_from_rfc3339(next)
            .map_err(|_| BridgeError::Invalid("nextRunAt must be RFC3339".into()))?;
    }
    db.execute(
        "UPDATE learning_jobs SET enabled=?2,cadence_minutes=?3,next_run_at=?4,run_budget_microusd=?5,mode=?6,updated_at=?7 WHERE id=?1",
        params![DEFAULT_JOB_ID, schedule.enabled, schedule.cadence_minutes, schedule.next_run_at, schedule.run_budget_microusd, schedule.mode, Utc::now().to_rfc3339()],
    )?;
    load_schedule(db)
}

pub fn learning_state(db: &Connection) -> Result<LearningState, BridgeError> {
    let latest_run = db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,created_at,completed_at FROM learning_job_runs ORDER BY created_at DESC,rowid DESC LIMIT 1",
        [],
        map_run,
    ).optional()?;
    Ok(LearningState {
        schedule: load_schedule(db)?,
        latest_run,
    })
}

pub fn run_due(db: &Connection, now: DateTime<Utc>) -> Result<Option<LearningRun>, BridgeError> {
    let schedule = load_schedule(db)?;
    if !schedule.enabled {
        return Ok(None);
    }
    let due = schedule
        .next_run_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|value| value <= now);
    if !due {
        return Ok(None);
    }
    let next = now + Duration::minutes(schedule.cadence_minutes);
    db.execute(
        "UPDATE learning_jobs SET next_run_at=?2,updated_at=?3 WHERE id=?1",
        params![DEFAULT_JOB_ID, next.to_rfc3339(), now.to_rfc3339()],
    )?;
    run_learning(db, LearningTriggerKind::InApp).map(Some)
}

pub fn register_trigger(
    db: &Connection,
    kind: LearningTriggerKind,
    registration_id: &str,
    credential_ref: Option<&str>,
) -> Result<(), BridgeError> {
    if !matches!(
        kind,
        LearningTriggerKind::Codex | LearningTriggerKind::Claude
    ) {
        return Err(BridgeError::Invalid(
            "only Codex and Claude require trigger registrations".into(),
        ));
    }
    if registration_id.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "trigger registration id cannot be empty".into(),
        ));
    }
    if credential_ref.is_some_and(|value| {
        let lower = value.to_ascii_lowercase();
        !(lower.starts_with("credential://")
            || lower.starts_with("keychain:")
            || lower.starts_with("secret-ref:"))
    }) {
        return Err(BridgeError::Invalid(
            "trigger credentials must be stored as credential references, never raw secrets".into(),
        ));
    }
    db.execute(
        "INSERT INTO learning_triggers(id,job_id,kind,registration_id,credential_ref,enabled,created_at)
         VALUES(?1,?2,?3,?4,?5,1,?6)
         ON CONFLICT(kind,registration_id) DO UPDATE SET credential_ref=excluded.credential_ref,enabled=1",
        params![Uuid::new_v4().to_string(), DEFAULT_JOB_ID, kind.as_str(), registration_id.trim(), credential_ref, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/learning','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Pune','Learning','bridge/learning','/tmp/learning-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','idle','reported')", []).unwrap();
        db
    }

    fn add_outcome(db: &Connection, index: i64) {
        let child = format!("child-{index}");
        let decision = format!("decision-{index}");
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES(?1,'w','codex','Child','completed','reported','parent')", params![child]).unwrap();
        db.execute("INSERT INTO router_decisions(id,workspace_id,parent_session_id,turn_id,task_family,mode,manual_override,decision,created_at) VALUES(?1,'w','parent',?2,'implementation','shadow',0,'{}','now')", params![decision, format!("turn-{index}")]).unwrap();
        db.execute("INSERT INTO router_outcomes(decision_id,child_session_id,candidate,succeeded,status,runtime_ms,normalized_cost,retry_count,human_intervention,recorded_at) VALUES(?1,?2,'codex:model',1,'completed',100,1000,0,0,'now')", params![decision, child]).unwrap();
    }

    #[test]
    fn duplicate_triggers_share_one_snapshot_run() {
        let db = database();
        let first = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        let second = run_learning(&db, LearningTriggerKind::Codex).unwrap();
        assert_eq!(first.id, second.id);
        assert!(second.duplicate);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learning_job_runs", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learning_trigger_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            2
        );
    }

    #[test]
    fn insufficient_evidence_is_auditable_noop() {
        let db = database();
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.status, LearningRunStatus::Noop);
        assert!(run.report.unwrap().reason.contains("insufficient evidence"));
    }

    #[test]
    fn recommendation_mode_never_promotes() {
        let db = database();
        for index in 0..MIN_EVIDENCE_SAMPLES {
            add_outcome(&db, index);
        }
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.status, LearningRunStatus::Completed);
        assert!(run.candidate_policy_version.is_some());
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='active'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='candidate'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn budget_and_secret_guards_are_deterministic() {
        let db = database();
        let mut schedule = load_schedule(&db).unwrap();
        schedule.run_budget_microusd = 0;
        update_schedule(&db, &schedule).unwrap();
        assert_eq!(
            run_learning(&db, LearningTriggerKind::Manual)
                .unwrap()
                .status,
            LearningRunStatus::Noop
        );
        assert!(register_trigger(
            &db,
            LearningTriggerKind::Claude,
            "routine",
            Some("Bearer raw-secret")
        )
        .is_err());
        register_trigger(
            &db,
            LearningTriggerKind::Claude,
            "routine",
            Some("keychain:bridge/claude-routine"),
        )
        .unwrap();
    }

    #[test]
    fn due_schedule_catches_up_once() {
        let db = database();
        let now = Utc::now();
        let schedule = LearningSchedule {
            job_id: DEFAULT_JOB_ID.into(),
            enabled: true,
            cadence_minutes: 60,
            next_run_at: Some((now - Duration::days(3)).to_rfc3339()),
            run_budget_microusd: 10_000,
            mode: "manual".into(),
        };
        update_schedule(&db, &schedule).unwrap();
        assert!(run_due(&db, now).unwrap().is_some());
        assert!(run_due(&db, now).unwrap().is_none());
        let next = DateTime::parse_from_rfc3339(
            load_schedule(&db).unwrap().next_run_at.as_deref().unwrap(),
        )
        .unwrap();
        assert!(next > now);
    }
}
