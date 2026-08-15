use crate::{routing_policy, BridgeError};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const DEFAULT_JOB_ID: &str = "default";
pub const MIN_EVIDENCE_SAMPLES: i64 = routing_policy::MIN_EVIDENCE_SAMPLES;
const LEASE_MINUTES: i64 = 15;
const CODEX_SCHEDULED_TASK_PROMPT: &str =
    include_str!("../../../docs/prompts/codex-learning-scheduled-task.md");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningTriggerKind {
    Manual,
    InApp,
    Codex,
    Claude,
    OpenCode,
}

impl LearningTriggerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::InApp => "in_app",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
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
    pub retry_rate_bps: Option<i64>,
    pub intervention_rate_bps: Option<i64>,
    pub average_confidence_bps: Option<i64>,
    pub cost_complete: bool,
    pub evaluated_spend_microusd: i64,
    pub evaluated_tokens: i64,
    #[serde(default = "default_evaluation_execution")]
    pub evaluation_execution: String,
    pub replay_passed: Option<bool>,
    pub promotion_status: String,
    pub policy_diff: serde_json::Value,
    pub recommendation_only: bool,
}

fn default_evaluation_execution() -> String {
    "not_run".into()
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
    pub lease_expires_at: Option<String>,
    pub replay_passed: Option<bool>,
    pub promotion_status: String,
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
    pub run_budget_tokens: i64,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningState {
    pub schedule: LearningSchedule,
    pub latest_run: Option<LearningRun>,
    pub active_policy_version: i64,
    pub canary_policy_version: Option<i64>,
}

fn active_policy_version(db: &Connection) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COALESCE(MAX(version),1) FROM routing_policies WHERE status IN ('active','canary')",
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
    run_id: Option<&str>,
    trigger_kind: LearningTriggerKind,
    registration_id: Option<&str>,
    result: &str,
    reason: Option<&str>,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO learning_trigger_events(id,run_id,trigger_kind,registration_id,result,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![Uuid::new_v4().to_string(), run_id, trigger_kind.as_str(), registration_id, result, reason, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[derive(Debug, Default)]
struct EvidenceSummary {
    count: i64,
    known_outcomes: i64,
    successes: i64,
    runtime_total: i64,
    runtime_reported: i64,
    cost_total: i64,
    cost_reported: i64,
    retries: i64,
    interventions: i64,
    confidence_total: i64,
    confidence_reported: i64,
}

impl EvidenceSummary {
    fn load(db: &Connection, boundary: i64) -> Result<Self, BridgeError> {
        db.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN success_state IN ('success','failure') THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN success_state='success' THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN runtime_ms IS NOT NULL THEN runtime_ms ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN runtime_ms IS NOT NULL THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN cost_microusd IS NOT NULL THEN cost_microusd ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN cost_microusd IS NOT NULL THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN retry_count>0 THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN human_intervention THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN confidence_bps IS NOT NULL THEN confidence_bps ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN confidence_bps IS NOT NULL THEN 1 ELSE 0 END),0)
             FROM router_outcomes WHERE rowid<=?1",
            params![boundary],
            |row| {
                Ok(Self {
                    count: row.get(0)?,
                    known_outcomes: row.get(1)?,
                    successes: row.get(2)?,
                    runtime_total: row.get(3)?,
                    runtime_reported: row.get(4)?,
                    cost_total: row.get(5)?,
                    cost_reported: row.get(6)?,
                    retries: row.get(7)?,
                    interventions: row.get(8)?,
                    confidence_total: row.get(9)?,
                    confidence_reported: row.get(10)?,
                })
            },
        )
        .map_err(BridgeError::from)
    }

    fn quality_bps(&self) -> Option<i64> {
        (self.known_outcomes > 0).then(|| self.successes * 10_000 / self.known_outcomes)
    }

    fn cost_per_success(&self) -> Option<i64> {
        (self.count > 0 && self.cost_reported == self.count && self.successes > 0)
            .then(|| self.cost_total / self.successes)
    }
}

fn active_policy_weights(db: &Connection, version: i64) -> Result<serde_json::Value, BridgeError> {
    let weights: String = db.query_row(
        "SELECT weights FROM routing_policies WHERE version=?1",
        params![version],
        |row| row.get(0),
    )?;
    serde_json::from_str(&weights).map_err(|error| BridgeError::Invalid(error.to_string()))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DeferredEvaluationSummary {
    pending: i64,
    reused_existing: i64,
}

impl DeferredEvaluationSummary {
    fn execution_status(&self) -> &'static str {
        if self.pending > 0 {
            "deferred"
        } else if self.reused_existing > 0 {
            "reused_existing_evidence"
        } else {
            "deterministic_only"
        }
    }
}

fn record_deferred_model_evaluations(
    db: &Connection,
    learning_run_id: &str,
    previous_boundary: i64,
    boundary: i64,
) -> Result<DeferredEvaluationSummary, BridgeError> {
    let mut summary = DeferredEvaluationSummary::default();
    let active_profile_version: Option<i64> = db
        .query_row(
            "SELECT active_version FROM model_setup_state WHERE id='default'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let mut evaluator_profiles = Vec::<(i64, String, String)>::new();
    if let Some(version) = active_profile_version {
        let mut statement = db.prepare(
            "SELECT version,provider,model FROM model_profiles
             WHERE version=?1 AND purpose IN ('evaluator','reviewer','verifier')
             ORDER BY CASE purpose WHEN 'evaluator' THEN 0 WHEN 'reviewer' THEN 1 ELSE 2 END",
        )?;
        evaluator_profiles = statement
            .query_map(params![version], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    let mut statement = db.prepare(
        "SELECT d.id,d.parent_session_id,d.actual_provider,o.runtime_ms,o.cost_microusd,o.retry_count,o.edit_count,o.override_signal,
                COALESCE((SELECT evidence_entry_ids FROM routing_evaluations e WHERE e.decision_id=d.id AND e.evaluator_kind='deterministic' ORDER BY e.created_at DESC LIMIT 1),'[]')
         FROM router_outcomes o JOIN router_decisions d ON d.id=o.decision_id
         WHERE o.rowid>?1 AND o.rowid<=?2 AND o.success_state='unknown'",
    )?;
    let rows = statement
        .query_map(params![previous_boundary, boundary], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (
        decision_id,
        parent_session_id,
        actual_provider,
        runtime_ms,
        cost,
        retries,
        edits,
        override_signal,
        deterministic_evidence_ids,
    ) in rows
    {
        let completed_eval: Option<(String, String, Option<String>, String)> = db
            .query_row(
                "SELECT c.status,c.verifier_family,c.output_digest,c.artifact_refs
             FROM eval_attempts a JOIN eval_check_runs c ON c.attempt_id=a.id
             WHERE a.session_id=?1 AND c.kind IN ('scrutiny','user_testing')
               AND c.status IN ('passed','failed') AND c.verifier_family IS NOT NULL
               AND (?2 IS NULL OR LOWER(c.verifier_family)<>LOWER(?2))
             ORDER BY c.completed_at DESC,c.rowid DESC LIMIT 1",
                params![parent_session_id, actual_provider],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let evaluator = evaluator_profiles.iter().find(|(_, provider, _)| {
            actual_provider
                .as_deref()
                .is_none_or(|actual| !provider.eq_ignore_ascii_case(actual))
        });
        let (evaluator_version, status, score_bps, confidence_bps, evidence_ids, source) =
            if let Some((check_status, verifier_family, output_digest, artifact_refs)) =
                completed_eval
            {
                let mut ids =
                    serde_json::from_str::<Vec<String>>(&artifact_refs).unwrap_or_default();
                if let Some(digest) = output_digest {
                    ids.push(format!("digest:{digest}"));
                }
                (
                    format!("independent:{verifier_family}:completion-v1"),
                    "completed",
                    Some(if check_status == "passed" {
                        10_000_i64
                    } else {
                        0_i64
                    }),
                    Some(9_000_i64),
                    serde_json::to_string(&ids)
                        .map_err(|error| BridgeError::Invalid(error.to_string()))?,
                    "independent_completion_verifier",
                )
            } else if let Some((version, provider, model)) = evaluator {
                (
                    format!("profile-v{version}:{provider}:{model}"),
                    "pending_bounded_model_eval",
                    None,
                    None,
                    deterministic_evidence_ids,
                    "deferred_profile",
                )
            } else {
                (
                    "none".into(),
                    "unavailable_independent_evaluator",
                    None,
                    None,
                    deterministic_evidence_ids,
                    "unavailable",
                )
            };
        db.execute(
            "INSERT INTO routing_evaluations(id,learning_run_id,decision_id,evaluator_kind,evaluator_version,score_bps,confidence_bps,evidence_entry_ids,bounded_metrics,status,created_at)
             VALUES(?1,?2,?3,'model_based',?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(id) DO UPDATE SET evaluator_version=excluded.evaluator_version,score_bps=excluded.score_bps,confidence_bps=excluded.confidence_bps,evidence_entry_ids=excluded.evidence_entry_ids,bounded_metrics=excluded.bounded_metrics,status=excluded.status,created_at=excluded.created_at",
            params![
                format!("model:{learning_run_id}:{decision_id}"),
                learning_run_id,
                decision_id,
                evaluator_version,
                score_bps,
                confidence_bps,
                evidence_ids,
                json!({
                    "runtimeMs": runtime_ms,
                    "costMicrousd": cost,
                    "retryCount": retries,
                    "editCount": edits,
                    "overrideSignal": override_signal,
                    "toolAccess": "none",
                    "transcriptIncluded": false,
                    "source": source,
                }).to_string(),
                status,
                Utc::now().to_rfc3339(),
            ],
        )?;
        match status {
            "pending_bounded_model_eval" => summary.pending += 1,
            "completed" => summary.reused_existing += 1,
            _ => {}
        }
    }
    Ok(summary)
}

fn fail_run(db: &Connection, id: &str, error: &BridgeError) -> Result<LearningRun, BridgeError> {
    let (boundary, base_policy_version, candidate_policy_version): (i64, i64, Option<i64>) = db.query_row(
        "SELECT evidence_boundary,base_policy_version,candidate_policy_version FROM learning_job_runs WHERE id=?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let summary = EvidenceSummary::load(db, boundary).unwrap_or_default();
    let report = LearningReport {
        reason: error.to_string(),
        evidence_boundary: boundary,
        evidence_count: summary.count,
        base_policy_version,
        candidate_policy_version: None,
        quality_bps: None,
        average_cost_microusd: None,
        average_latency_ms: None,
        retry_rate_bps: None,
        intervention_rate_bps: None,
        average_confidence_bps: None,
        cost_complete: false,
        evaluated_spend_microusd: 0,
        evaluated_tokens: 0,
        evaluation_execution: "not_run".into(),
        replay_passed: None,
        promotion_status: "failed".into(),
        policy_diff: json!({}),
        recommendation_only: true,
    };
    let transaction = db.unchecked_transaction()?;
    if let Some(candidate) = candidate_policy_version {
        transaction.execute(
            "UPDATE routing_policies SET status='abandoned' WHERE version=?1 AND status='candidate'",
            params![candidate],
        )?;
    }
    transaction.execute(
        "UPDATE learning_job_runs SET status='failed',report=?2,lease_owner=NULL,lease_expires_at=NULL,promotion_status='failed',completed_at=?3 WHERE id=?1",
        params![id, serde_json::to_string(&report).map_err(|error| BridgeError::Invalid(error.to_string()))?, Utc::now().to_rfc3339()],
    )?;
    transaction.commit()?;
    load_run(db, id)?.ok_or_else(|| BridgeError::Invalid("failed learning run disappeared".into()))
}

fn persist_candidate_policy(
    db: &Connection,
    run_id: &str,
    base_version: i64,
    boundary: i64,
    candidate: &routing_policy::CandidatePolicy,
) -> Result<i64, BridgeError> {
    let replay_json = serde_json::to_string(&candidate.replay)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let transaction = db.unchecked_transaction()?;
    let next_version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(version),0)+1 FROM routing_policies",
        [],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO routing_policies(version,status,predecessor,weights,thresholds,replay_report,created_reason,created_at)
         VALUES(?1,'candidate',?2,?3,?4,?5,?6,?7)",
        params![next_version, base_version, candidate.weights.to_string(), candidate.thresholds.to_string(), replay_json, format!("learning candidate from frozen evidence boundary {boundary}"), Utc::now().to_rfc3339()],
    )?;
    let linked = transaction.execute(
        "UPDATE learning_job_runs SET candidate_policy_version=?2 WHERE id=?1 AND status='running'",
        params![run_id, next_version],
    )?;
    if linked != 1 {
        return Err(BridgeError::Invalid(
            "candidate policy could not be linked to its running learning job".into(),
        ));
    }
    transaction.commit()?;
    Ok(next_version)
}

pub fn run_learning(
    db: &Connection,
    trigger_kind: LearningTriggerKind,
) -> Result<LearningRun, BridgeError> {
    settle_canary(db)?;
    let boundary = evidence_boundary(db)?;
    let base_version = active_policy_version(db)?;
    let key = format!("{DEFAULT_JOB_ID}:{boundary}:{base_version}");
    let now = Utc::now();
    let lease_owner = Uuid::new_v4().to_string();
    if let Some(mut active) = load_active_run(db)? {
        if active.idempotency_key != key {
            let lease_expired = active
                .lease_expires_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .is_some_and(|expires| expires <= now);
            if lease_expired {
                let previous_expiry = active.lease_expires_at.clone().ok_or_else(|| {
                    BridgeError::Invalid("expired learning lease lost its expiry boundary".into())
                })?;
                let acquired = db.execute(
                    "UPDATE learning_job_runs SET lease_owner=?2,lease_expires_at=?3
                     WHERE id=?1 AND status='running' AND lease_expires_at=?4",
                    params![
                        active.id,
                        lease_owner,
                        (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339(),
                        previous_expiry
                    ],
                )?;
                if acquired == 1 {
                    record_trigger_event(
                        db,
                        Some(&active.id),
                        trigger_kind,
                        None,
                        "lease_recovered",
                        Some("the prior durable lease expired; the frozen snapshot was resumed before newer evidence"),
                    )?;
                    return process_run(
                        db,
                        &active.id,
                        active.evidence_boundary,
                        active.base_policy_version,
                    )
                    .or_else(|error| fail_run(db, &active.id, &error));
                }
                active = load_active_run(db)?.ok_or_else(|| {
                    BridgeError::Invalid("recovered learning lease disappeared".into())
                })?;
            }
            record_trigger_event(
                db,
                Some(&active.id),
                trigger_kind,
                None,
                "duplicate_noop",
                Some("another snapshot is already protected by the durable job lease"),
            )?;
            active.duplicate = true;
            return Ok(active);
        }
    }
    if let Some(mut existing) = load_run_by_key(db, &key)? {
        let lease_expired = existing.status == LearningRunStatus::Running
            && existing
                .lease_expires_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .is_some_and(|expires| expires <= now);
        if lease_expired {
            let previous_expiry = existing.lease_expires_at.clone().ok_or_else(|| {
                BridgeError::Invalid("expired learning lease lost its expiry boundary".into())
            })?;
            let acquired = db.execute(
                "UPDATE learning_job_runs SET lease_owner=?2,lease_expires_at=?3
                 WHERE id=?1 AND status='running' AND lease_expires_at=?4",
                params![
                    existing.id,
                    lease_owner,
                    (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339(),
                    previous_expiry
                ],
            )?;
            if acquired == 0 {
                let mut winner = load_run_by_key(db, &key)?.ok_or_else(|| {
                    BridgeError::Invalid("recovered learning lease disappeared".into())
                })?;
                record_trigger_event(
                    db,
                    Some(&winner.id),
                    trigger_kind,
                    None,
                    "duplicate_noop",
                    Some("another trigger recovered the expired durable lease"),
                )?;
                winner.duplicate = true;
                return Ok(winner);
            }
            record_trigger_event(
                db,
                Some(&existing.id),
                trigger_kind,
                None,
                "lease_recovered",
                Some("the prior durable lease expired; the frozen snapshot was resumed"),
            )?;
            return process_run(db, &existing.id, boundary, base_version)
                .or_else(|error| fail_run(db, &existing.id, &error));
        }
        record_trigger_event(
            db,
            Some(&existing.id),
            trigger_kind,
            None,
            "duplicate_noop",
            Some("the snapshot idempotency key already exists"),
        )?;
        existing.duplicate = true;
        return Ok(existing);
    }

    let id = Uuid::new_v4().to_string();
    let created_at = now.to_rfc3339();
    let inserted = db.execute(
        "INSERT OR IGNORE INTO learning_job_runs(id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,lease_owner,lease_expires_at,snapshot_frozen_at,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,'running',?7,?8,?9,?9)",
        params![id, DEFAULT_JOB_ID, trigger_kind.as_str(), key, boundary, base_version, lease_owner, (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339(), created_at],
    )?;
    if inserted == 0 {
        let mut existing = match load_run_by_key(db, &key)? {
            Some(existing) => existing,
            None => load_active_run(db)?
                .ok_or_else(|| BridgeError::Invalid("learning run lease claim was lost".into()))?,
        };
        record_trigger_event(
            db,
            Some(&existing.id),
            trigger_kind,
            None,
            "duplicate_noop",
            Some("a concurrent trigger acquired the durable lease"),
        )?;
        existing.duplicate = true;
        return Ok(existing);
    }
    record_trigger_event(
        db,
        Some(&id),
        trigger_kind,
        None,
        "acquired",
        Some("durable lease acquired and evidence snapshot frozen"),
    )?;
    process_run(db, &id, boundary, base_version).or_else(|error| fail_run(db, &id, &error))
}

fn process_run(
    db: &Connection,
    id: &str,
    boundary: i64,
    base_version: i64,
) -> Result<LearningRun, BridgeError> {
    let schedule = load_schedule(db)?;
    let summary = EvidenceSummary::load(db, boundary)?;
    let previous_boundary: i64 = db.query_row(
        "SELECT last_evidence_boundary FROM learning_jobs WHERE id=?1",
        params![DEFAULT_JOB_ID],
        |row| row.get(0),
    )?;
    let new_evidence_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM router_outcomes WHERE rowid>?1 AND rowid<=?2",
        params![previous_boundary, boundary],
        |row| row.get(0),
    )?;
    let mut candidate_policy_version = None;
    let mut replay_passed = None;
    let mut promotion_status = "not_requested".to_owned();
    let mut policy_diff = json!({});
    let mut consumed_evidence = false;
    let mut evaluation_execution = "not_run".to_owned();
    let canary_pending = schedule.mode == "automatic"
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM routing_policies WHERE status='canary')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
    let (status, reason) = if schedule.run_budget_microusd <= 0 || schedule.run_budget_tokens <= 0 {
        (
            LearningRunStatus::Noop,
            "learning spend/token budget exhausted".to_owned(),
        )
    } else if canary_pending {
        (
            LearningRunStatus::Noop,
            "guarded canary is still collecting outcomes; no second automatic promotion was created".to_owned(),
        )
    } else if summary.count < MIN_EVIDENCE_SAMPLES {
        (
            LearningRunStatus::Noop,
            format!(
                "insufficient evidence: {}/{MIN_EVIDENCE_SAMPLES} outcomes",
                summary.count
            ),
        )
    } else if new_evidence_count < MIN_EVIDENCE_SAMPLES {
        (
            LearningRunStatus::Noop,
            format!("insufficient new evidence: {new_evidence_count}/{MIN_EVIDENCE_SAMPLES} outcomes since boundary {previous_boundary}"),
        )
    } else {
        let evaluation_summary =
            record_deferred_model_evaluations(db, id, previous_boundary, boundary)?;
        evaluation_execution = evaluation_summary.execution_status().into();
        consumed_evidence = true;
        let base_weights = active_policy_weights(db, base_version)?;
        match routing_policy::build_candidate(db, boundary, &base_weights)? {
            None => (LearningRunStatus::Noop, "insufficient value: no supported policy change met the sample and confidence thresholds".into()),
            Some(candidate) if !candidate.replay.passed => {
                replay_passed = Some(false);
                policy_diff = serde_json::to_value(&candidate.replay).map_err(|error| BridgeError::Invalid(error.to_string()))?;
                (LearningRunStatus::Noop, format!("held-out replay rejected the candidate: {}", candidate.replay.reasons.join("; ")))
            }
            Some(candidate) => {
                replay_passed = Some(true);
                let next_version = persist_candidate_policy(db, id, base_version, boundary, &candidate)?;
                candidate_policy_version = Some(next_version);
                policy_diff = json!({
                    "weights": candidate.weights,
                    "thresholds": candidate.thresholds,
                    "replay": candidate.replay,
                    "evidenceGroups": candidate.evidence_groups,
                    "guardrails": "deterministic_eligibility_unchanged",
                });
                match schedule.mode.as_str() {
                    "manual" => promotion_status = "recommended".into(),
                    "ask" => promotion_status = "awaiting_approval".into(),
                    "automatic" => {
                        let cancellation_requested: bool = db.query_row(
                            "SELECT cancellation_requested FROM learning_job_runs WHERE id=?1",
                            params![id],
                            |row| row.get(0),
                        )?;
                        if cancellation_requested {
                            return cancel_run(db, id);
                        }
                        promote_candidate(db, id, next_version, "automatic", true)?;
                        promotion_status = "canary".into();
                    }
                    _ => return Err(BridgeError::Invalid("unsupported learning mode".into())),
                }
                (LearningRunStatus::Completed, match schedule.mode.as_str() {
                    "manual" => "candidate policy recommended",
                    "ask" => "candidate policy awaits approval",
                    "automatic" => "candidate policy promoted to guarded canary",
                    _ => unreachable!(),
                }.into())
            }
        }
    };
    let report = LearningReport {
        reason,
        evidence_boundary: boundary,
        evidence_count: summary.count,
        base_policy_version: base_version,
        candidate_policy_version,
        quality_bps: summary.quality_bps(),
        average_cost_microusd: summary.cost_per_success(),
        average_latency_ms: (summary.runtime_reported > 0)
            .then(|| summary.runtime_total / summary.runtime_reported),
        retry_rate_bps: (summary.count > 0).then(|| summary.retries * 10_000 / summary.count),
        intervention_rate_bps: (summary.count > 0)
            .then(|| summary.interventions * 10_000 / summary.count),
        average_confidence_bps: (summary.confidence_reported > 0)
            .then(|| summary.confidence_total / summary.confidence_reported),
        cost_complete: summary.count > 0 && summary.cost_reported == summary.count,
        evaluated_spend_microusd: 0,
        evaluated_tokens: 0,
        evaluation_execution,
        replay_passed,
        promotion_status: promotion_status.clone(),
        policy_diff,
        recommendation_only: schedule.mode != "automatic",
    };
    let completed_at = Utc::now().to_rfc3339();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "UPDATE learning_job_runs SET status=?2,report=?3,candidate_policy_version=?4,evaluated_spend_microusd=0,evaluated_tokens=0,replay_passed=?5,promotion_status=?6,lease_owner=NULL,lease_expires_at=NULL,completed_at=?7 WHERE id=?1",
        params![id, status.as_str(), serde_json::to_string(&report).map_err(|error| BridgeError::Invalid(error.to_string()))?, candidate_policy_version, replay_passed, promotion_status, completed_at],
    )?;
    if consumed_evidence {
        transaction.execute(
            "UPDATE learning_jobs SET last_evidence_boundary=MAX(last_evidence_boundary,?2),updated_at=?3 WHERE id=?1",
            params![DEFAULT_JOB_ID, boundary, completed_at],
        )?;
    }
    transaction.commit()?;
    load_run(db, &id)?.ok_or_else(|| BridgeError::Invalid("learning run disappeared".into()))
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearningRun> {
    let trigger = match row.get::<_, String>(2)?.as_str() {
        "manual" => LearningTriggerKind::Manual,
        "in_app" => LearningTriggerKind::InApp,
        "codex" => LearningTriggerKind::Codex,
        "claude" => LearningTriggerKind::Claude,
        "opencode" => LearningTriggerKind::OpenCode,
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
        lease_expires_at: row.get(10)?,
        replay_passed: row.get(11)?,
        promotion_status: row.get(12)?,
        duplicate: false,
        created_at: row.get(13)?,
        completed_at: row.get(14)?,
    })
}

pub fn load_run(db: &Connection, id: &str) -> Result<Option<LearningRun>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,lease_expires_at,replay_passed,promotion_status,created_at,completed_at FROM learning_job_runs WHERE id=?1",
        params![id],
        map_run,
    ).optional()?)
}

fn load_run_by_key(db: &Connection, key: &str) -> Result<Option<LearningRun>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,lease_expires_at,replay_passed,promotion_status,created_at,completed_at FROM learning_job_runs WHERE idempotency_key=?1",
        params![key],
        map_run,
    ).optional()?)
}

fn load_active_run(db: &Connection) -> Result<Option<LearningRun>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,lease_expires_at,replay_passed,promotion_status,created_at,completed_at
         FROM learning_job_runs WHERE job_id=?1 AND status IN ('queued','running') ORDER BY created_at LIMIT 1",
        params![DEFAULT_JOB_ID],
        map_run,
    ).optional()?)
}

pub fn cancel_run(db: &Connection, id: &str) -> Result<LearningRun, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let candidate: Option<i64> = transaction
        .query_row(
            "SELECT candidate_policy_version FROM learning_job_runs
             WHERE id=?1 AND (status IN ('queued','running') OR promotion_status IN ('recommended','awaiting_approval'))",
            params![id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let updated = transaction.execute(
        "UPDATE learning_job_runs SET cancellation_requested=1,status='cancelled',promotion_status='cancelled',lease_owner=NULL,lease_expires_at=NULL,completed_at=?2
         WHERE id=?1 AND (status IN ('queued','running') OR promotion_status IN ('recommended','awaiting_approval'))",
        params![id, Utc::now().to_rfc3339()],
    )?;
    if updated == 1 {
        if let Some(candidate) = candidate {
            transaction.execute(
                "UPDATE routing_policies SET status='abandoned' WHERE version=?1 AND status='candidate'",
                params![candidate],
            )?;
        }
        transaction.commit()?;
    }
    load_run(db, id)?.ok_or_else(|| BridgeError::Invalid(format!("learning run {id} not found")))
}

fn promote_candidate(
    db: &Connection,
    run_id: &str,
    candidate_version: i64,
    actor: &str,
    canary: bool,
) -> Result<(), BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let (current, current_status): (i64, String) = transaction.query_row(
        "SELECT version,status FROM routing_policies WHERE status IN ('active','canary') LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if current_status == "canary" {
        return Err(BridgeError::Invalid(
            "an existing canary must settle or roll back before another policy can promote".into(),
        ));
    }
    let (predecessor, replay): (Option<i64>, Option<String>) = transaction.query_row(
        "SELECT predecessor,replay_report FROM routing_policies WHERE version=?1 AND status='candidate'",
        params![candidate_version],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if predecessor != Some(current) {
        return Err(BridgeError::Invalid(
            "candidate predecessor is stale; run learning again against the active policy".into(),
        ));
    }
    let cancellation_requested: bool = transaction.query_row(
        "SELECT cancellation_requested FROM learning_job_runs WHERE id=?1",
        params![run_id],
        |row| row.get(0),
    )?;
    if cancellation_requested {
        return Err(BridgeError::Invalid(
            "cancelled learning runs cannot promote policies".into(),
        ));
    }
    let boundary: i64 = transaction.query_row(
        "SELECT evidence_boundary FROM learning_job_runs WHERE id=?1",
        params![run_id],
        |row| row.get(0),
    )?;
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "UPDATE routing_policies SET status='archived' WHERE version=?1",
        params![current],
    )?;
    let next_status = if canary { "canary" } else { "active" };
    transaction.execute(
        "UPDATE routing_policies SET status=?2,promoted_at=?3,activation_boundary=?4 WHERE version=?1",
        params![candidate_version, next_status, now, boundary],
    )?;
    transaction.execute(
        "INSERT INTO routing_policy_promotions(id,from_version,to_version,learning_run_id,action,actor,explanation,replay_report,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![Uuid::new_v4().to_string(), current, candidate_version, run_id, if canary { "canary_started" } else { "promoted" }, actor, if canary { "held-out replay passed; guarded canary started" } else { "held-out replay passed and the user approved promotion" }, replay, now],
    )?;
    transaction.execute(
        "UPDATE learning_job_runs SET promotion_status=?2 WHERE id=?1",
        params![run_id, if canary { "canary" } else { "promoted" }],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn approve_run(db: &Connection, id: &str) -> Result<LearningRun, BridgeError> {
    let schedule = load_schedule(db)?;
    if schedule.mode != "ask" {
        return Err(BridgeError::Invalid(
            "policy approval is available only while learning mode is Ask".into(),
        ));
    }
    let run = load_run(db, id)?
        .ok_or_else(|| BridgeError::Invalid(format!("learning run {id} not found")))?;
    if run.status != LearningRunStatus::Completed
        || run.replay_passed != Some(true)
        || run.promotion_status != "awaiting_approval"
        || run.cancellation_requested
    {
        return Err(BridgeError::Invalid(
            "only a completed, replay-approved, non-cancelled Ask candidate can be promoted".into(),
        ));
    }
    let candidate = run
        .candidate_policy_version
        .ok_or_else(|| BridgeError::Invalid("learning run has no candidate policy".into()))?;
    promote_candidate(db, id, candidate, "user_approval", false)?;
    db.execute(
        "UPDATE learning_job_runs SET promotion_status='promoted' WHERE id=?1",
        params![id],
    )?;
    load_run(db, id)?
        .ok_or_else(|| BridgeError::Invalid("approved learning run disappeared".into()))
}

fn rollback_policy_internal(
    db: &Connection,
    target_version: i64,
    actor: &str,
    explanation: &str,
    learning_run_id: Option<&str>,
) -> Result<i64, BridgeError> {
    if explanation.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "rollback explanation cannot be empty".into(),
        ));
    }
    let transaction = db.unchecked_transaction()?;
    let current: i64 = transaction.query_row(
        "SELECT version FROM routing_policies WHERE status IN ('active','canary') LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    if current == target_version {
        return Err(BridgeError::Invalid(
            "target policy is already active".into(),
        ));
    }
    let (target_status, weights, thresholds, replay): (String, String, String, Option<String>) =
        transaction.query_row(
            "SELECT status,weights,thresholds,replay_report FROM routing_policies WHERE version=?1",
            params![target_version],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if !matches!(target_status.as_str(), "archived" | "rolled_back") {
        return Err(BridgeError::Invalid(
            "rollback target must be a previously active policy version".into(),
        ));
    }
    let next_version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(version),0)+1 FROM routing_policies",
        [],
        |row| row.get(0),
    )?;
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "UPDATE routing_policies SET status=CASE WHEN status='canary' THEN 'rolled_back' ELSE 'archived' END WHERE version=?1",
        params![current],
    )?;
    transaction.execute(
        "INSERT INTO routing_policies(version,status,predecessor,rollback_of,weights,thresholds,replay_report,created_reason,created_at,promoted_at,activation_boundary)
         VALUES(?1,'active',?2,?3,?4,?5,?6,?7,?8,?8,(SELECT COALESCE(MAX(rowid),0) FROM router_outcomes))",
        params![next_version, current, target_version, weights, thresholds, replay, format!("rollback to policy v{target_version}: {}", explanation.trim()), now],
    )?;
    transaction.execute(
        "INSERT INTO routing_policy_promotions(id,from_version,to_version,learning_run_id,action,actor,explanation,replay_report,created_at)
         VALUES(?1,?2,?3,?4,'rollback',?5,?6,?7,?8)",
        params![Uuid::new_v4().to_string(), current, next_version, learning_run_id, actor, explanation.trim(), replay, now],
    )?;
    transaction.commit()?;
    Ok(next_version)
}

pub fn rollback_policy(
    db: &Connection,
    target_version: i64,
    explanation: &str,
) -> Result<i64, BridgeError> {
    rollback_policy_internal(db, target_version, "user", explanation, None)
}

fn settle_canary(db: &Connection) -> Result<(), BridgeError> {
    let canary: Option<(i64, i64, i64, String)> = db
        .query_row(
            "SELECT version,predecessor,COALESCE(activation_boundary,0),replay_report FROM routing_policies WHERE status='canary' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((version, predecessor, boundary, replay_json)) = canary else {
        return Ok(());
    };
    let (count, known, successes, cost_total, cost_reported, latency_total, retries, interventions): (i64, i64, i64, i64, i64, i64, i64, i64) = db.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN o.success_state IN ('success','failure') THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN o.success_state='success' THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN o.cost_microusd IS NOT NULL THEN o.cost_microusd ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN o.cost_microusd IS NOT NULL THEN 1 ELSE 0 END),0),
                COALESCE(SUM(o.runtime_ms),0),
                COALESCE(SUM(CASE WHEN o.retry_count>0 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN o.human_intervention THEN 1 ELSE 0 END),0)
         FROM router_outcomes o JOIN router_decisions d ON d.id=o.decision_id
         WHERE o.rowid>?1 AND d.policy_version=?2 AND d.mode='autonomous' AND d.executed_candidate=d.recommended_candidate",
        params![boundary, version],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
    )?;
    if count < MIN_EVIDENCE_SAMPLES {
        return Ok(());
    }
    let expected: routing_policy::PolicyReplayReport = serde_json::from_str(&replay_json)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let quality = (known == count && known > 0).then(|| successes * 10_000 / known);
    let cost = (cost_reported == count && successes > 0).then(|| cost_total / successes);
    let latency = (count > 0).then(|| latency_total / count);
    let retry = retries * 10_000 / count;
    let intervention = interventions * 10_000 / count;
    let regressed = quality
        .zip(expected.candidate.quality_bps)
        .is_some_and(|(actual, expected)| actual + 500 < expected)
        || cost
            .zip(expected.candidate.cost_per_success_microusd)
            .is_some_and(|(actual, expected)| actual * 10_000 > expected * 11_000)
        || latency
            .zip(expected.candidate.average_latency_ms)
            .is_some_and(|(actual, expected)| actual * 10_000 > expected * 12_000)
        || expected
            .candidate
            .retry_rate_bps
            .is_some_and(|expected| retry > expected + 500)
        || expected
            .candidate
            .intervention_rate_bps
            .is_some_and(|expected| intervention > expected + 500);
    let required_metrics_available = expected
        .candidate
        .quality_bps
        .is_none_or(|_| quality.is_some())
        && expected
            .candidate
            .cost_per_success_microusd
            .is_none_or(|_| cost.is_some())
        && expected
            .candidate
            .average_latency_ms
            .is_none_or(|_| latency.is_some());
    if !regressed && !required_metrics_available {
        return Ok(());
    }
    if regressed {
        rollback_policy_internal(
            db,
            predecessor,
            "canary_guardrail",
            "automatic canary regressed against held-out replay guardrails",
            None,
        )?;
    } else {
        let transaction = db.unchecked_transaction()?;
        transaction.execute(
            "UPDATE routing_policies SET status='active' WHERE version=?1 AND status='canary'",
            params![version],
        )?;
        transaction.execute(
            "INSERT INTO routing_policy_promotions(id,from_version,to_version,action,actor,explanation,replay_report,created_at)
             VALUES(?1,?2,?2,'canary_completed','canary_guardrail','canary evidence satisfied every regression guardrail',?3,?4)",
            params![Uuid::new_v4().to_string(), version, replay_json, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

pub fn load_schedule(db: &Connection) -> Result<LearningSchedule, BridgeError> {
    Ok(db.query_row(
        "SELECT id,enabled,cadence_minutes,next_run_at,run_budget_microusd,run_budget_tokens,mode FROM learning_jobs WHERE id=?1",
        params![DEFAULT_JOB_ID],
        |row| Ok(LearningSchedule { job_id: row.get(0)?, enabled: row.get(1)?, cadence_minutes: row.get(2)?, next_run_at: row.get(3)?, run_budget_microusd: row.get(4)?, run_budget_tokens: row.get(5)?, mode: row.get(6)? }),
    )?)
}

pub fn update_schedule(
    db: &Connection,
    schedule: &LearningSchedule,
) -> Result<LearningSchedule, BridgeError> {
    if schedule.job_id != DEFAULT_JOB_ID
        || schedule.cadence_minutes < 15
        || schedule.run_budget_microusd < 0
        || schedule.run_budget_tokens < 0
    {
        return Err(BridgeError::Invalid("learning schedule requires the default job, cadence >= 15 minutes, and non-negative spend/token budgets".into()));
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
        "UPDATE learning_jobs SET enabled=?2,cadence_minutes=?3,next_run_at=?4,run_budget_microusd=?5,run_budget_tokens=?6,mode=?7,updated_at=?8 WHERE id=?1",
        params![DEFAULT_JOB_ID, schedule.enabled, schedule.cadence_minutes, schedule.next_run_at, schedule.run_budget_microusd, schedule.run_budget_tokens, schedule.mode, Utc::now().to_rfc3339()],
    )?;
    load_schedule(db)
}

pub fn learning_state(db: &Connection) -> Result<LearningState, BridgeError> {
    let latest_run = db.query_row(
        "SELECT id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,report,candidate_policy_version,cancellation_requested,lease_expires_at,replay_passed,promotion_status,created_at,completed_at FROM learning_job_runs ORDER BY created_at DESC,rowid DESC LIMIT 1",
        [],
        map_run,
    ).optional()?;
    Ok(LearningState {
        schedule: load_schedule(db)?,
        latest_run,
        active_policy_version: active_policy_version(db)?,
        canary_policy_version: db
            .query_row(
                "SELECT version FROM routing_policies WHERE status='canary' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?,
    })
}

pub fn run_due(db: &Connection, now: DateTime<Utc>) -> Result<Option<LearningRun>, BridgeError> {
    let schedule = load_schedule(db)?;
    if !schedule.enabled {
        return Ok(None);
    }
    let app_idle: bool = db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sessions WHERE status IN ('working','waiting'))",
        [],
        |row| row.get(0),
    )?;
    if !app_idle {
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
    register_trigger_with_expiry(db, kind, registration_id, credential_ref, None)
}

pub fn register_trigger_with_expiry(
    db: &Connection,
    kind: LearningTriggerKind,
    registration_id: &str,
    credential_ref: Option<&str>,
    expires_at: Option<&str>,
) -> Result<(), BridgeError> {
    if !matches!(
        kind,
        LearningTriggerKind::Codex | LearningTriggerKind::Claude | LearningTriggerKind::OpenCode
    ) {
        return Err(BridgeError::Invalid(
            "only Codex, Claude, and OpenCode require trigger registrations".into(),
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
    if let Some(expires_at) = expires_at {
        DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| BridgeError::Invalid("trigger expiry must be RFC3339".into()))?;
    }
    let enabled = !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM learning_triggers WHERE enabled=1 AND kind IN ('codex','claude','opencode'))",
        [],
        |row| row.get::<_, bool>(0),
    )? || db.query_row(
        "SELECT EXISTS(SELECT 1 FROM learning_triggers WHERE kind=?1 AND registration_id=?2 AND enabled=1)",
        params![kind.as_str(), registration_id.trim()],
        |row| row.get::<_, bool>(0),
    )?;
    let auth_digest = credential_ref.map(|reference| {
        format!(
            "{:x}",
            Sha256::digest(
                format!("{}:{}:{reference}", kind.as_str(), registration_id.trim()).as_bytes()
            )
        )
    });
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO learning_triggers(id,job_id,kind,registration_id,credential_ref,auth_digest,enabled,expires_at,experimental,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)
         ON CONFLICT(kind,registration_id) DO UPDATE SET credential_ref=excluded.credential_ref,auth_digest=excluded.auth_digest,expires_at=excluded.expires_at,experimental=excluded.experimental,updated_at=excluded.updated_at",
        params![Uuid::new_v4().to_string(), DEFAULT_JOB_ID, kind.as_str(), registration_id.trim(), credential_ref, auth_digest, enabled, expires_at, kind == LearningTriggerKind::Claude, now],
    )?;
    Ok(())
}

pub fn enable_trigger(
    db: &Connection,
    kind: LearningTriggerKind,
    registration_id: &str,
) -> Result<(), BridgeError> {
    if !matches!(
        kind,
        LearningTriggerKind::Codex | LearningTriggerKind::Claude | LearningTriggerKind::OpenCode
    ) {
        return Err(BridgeError::Invalid(
            "only external trigger adapters can be enabled".into(),
        ));
    }
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "UPDATE learning_triggers SET enabled=0,updated_at=?1 WHERE kind IN ('codex','claude','opencode')",
        params![Utc::now().to_rfc3339()],
    )?;
    let updated = transaction.execute(
        "UPDATE learning_triggers SET enabled=1,updated_at=?3 WHERE kind=?1 AND registration_id=?2",
        params![kind.as_str(), registration_id, Utc::now().to_rfc3339()],
    )?;
    if updated != 1 {
        return Err(BridgeError::Invalid(
            "external trigger registration does not exist".into(),
        ));
    }
    transaction.commit()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalTriggerResult {
    pub accepted: bool,
    pub reason: String,
    pub run: Option<LearningRun>,
}

pub fn trigger_instructions(
    kind: LearningTriggerKind,
    database_path: &str,
    registration_id: &str,
) -> Result<String, BridgeError> {
    if registration_id.trim().is_empty() || database_path.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "trigger instructions require a database path and registration id".into(),
        ));
    }
    match kind {
        LearningTriggerKind::Codex => Ok(CODEX_SCHEDULED_TASK_PROMPT
            .replace("{{BRIDGE_DATABASE}}", database_path.trim())
            .replace("{{REGISTRATION_ID}}", registration_id.trim())),
        LearningTriggerKind::Claude => Ok(format!(
            "Run `bridge learning run --database \"{}\" --trigger claude:{}` as a local Claude Desktop scheduled task. Treat it as a wake-up only. Do not upload Bridge's SQLite/WAL files or promote policy. Cloud Routine support is experimental and requires a future Bridge-owned authenticated endpoint.",
            database_path.trim(),
            registration_id.trim()
        )),
        LearningTriggerKind::OpenCode => Ok(format!(
            "Run `bridge learning run --database \"{}\" --trigger opencode:{}` as a local OpenCode scheduled command. Treat it as a wake-up only. Do not upload Bridge's SQLite/WAL files or promote policy.",
            database_path.trim(),
            registration_id.trim()
        )),
        LearningTriggerKind::Manual | LearningTriggerKind::InApp => Err(BridgeError::Invalid(
            "only external providers have scheduled-task instructions".into(),
        )),
    }
}

fn run_external_trigger(
    db: &Connection,
    kind: LearningTriggerKind,
    registration_id: &str,
    credential_ref: Option<&str>,
) -> Result<ExternalTriggerResult, BridgeError> {
    let registration: Option<(bool, Option<String>, Option<String>, Option<String>)> = db
        .query_row(
            "SELECT enabled,expires_at,auth_digest,credential_ref FROM learning_triggers WHERE kind=?1 AND registration_id=?2",
            params![kind.as_str(), registration_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let reject = |result: &str, reason: &str| -> Result<ExternalTriggerResult, BridgeError> {
        record_trigger_event(db, None, kind, Some(registration_id), result, Some(reason))?;
        Ok(ExternalTriggerResult {
            accepted: false,
            reason: reason.into(),
            run: None,
        })
    };
    let Some((enabled, expires_at, expected_digest, _stored_reference)) = registration else {
        return reject(
            "unauthorized_noop",
            "external trigger registration does not exist",
        );
    };
    if !enabled {
        return reject("disabled_noop", "external trigger registration is disabled");
    }
    if expires_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|expires| expires <= Utc::now())
    {
        return reject("expired_noop", "external trigger registration expired");
    }
    let supplied_digest = credential_ref.map(|reference| {
        format!(
            "{:x}",
            Sha256::digest(format!("{}:{registration_id}:{reference}", kind.as_str()).as_bytes())
        )
    });
    if expected_digest != supplied_digest {
        return reject(
            "unauthorized_noop",
            "external trigger credential reference did not match",
        );
    }
    let run = run_learning(db, kind)?;
    record_trigger_event(
        db,
        Some(&run.id),
        kind,
        Some(registration_id),
        if run.duplicate {
            "duplicate_noop"
        } else {
            "accepted"
        },
        Some(if run.duplicate {
            "snapshot already claimed"
        } else {
            "registered external wake-up accepted"
        }),
    )?;
    Ok(ExternalTriggerResult {
        accepted: true,
        reason: if run.duplicate {
            "snapshot already claimed".into()
        } else {
            "registered external wake-up accepted".into()
        },
        run: Some(run),
    })
}

pub fn run_database(
    database_path: &std::path::Path,
    trigger: &str,
    credential_ref: Option<&str>,
) -> Result<ExternalTriggerResult, BridgeError> {
    let (kind, registration_id) = parse_trigger(trigger)?;
    if !database_path.is_file() {
        return Err(BridgeError::Invalid(format!(
            "Bridge database does not exist: {}",
            database_path.display()
        )));
    }
    let db = open_existing_database(database_path)?;
    if let Some(registration_id) = registration_id {
        run_external_trigger(&db, kind, &registration_id, credential_ref)
    } else if credential_ref.is_some() {
        Err(BridgeError::Invalid(
            "credential references are accepted only for Codex, Claude, or OpenCode triggers"
                .into(),
        ))
    } else {
        let run = run_learning(&db, kind)?;
        Ok(ExternalTriggerResult {
            accepted: true,
            reason: "local learning trigger accepted".into(),
            run: Some(run),
        })
    }
}

fn open_existing_database(database_path: &std::path::Path) -> Result<Connection, BridgeError> {
    let db = Connection::open(database_path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(db)
}

pub fn run_local_database(
    database_path: &std::path::Path,
    trigger_kind: LearningTriggerKind,
) -> Result<LearningRun, BridgeError> {
    if !database_path.is_file() {
        return Err(BridgeError::Invalid(format!(
            "Bridge database does not exist: {}",
            database_path.display()
        )));
    }
    run_learning(&open_existing_database(database_path)?, trigger_kind)
}

pub fn run_due_database(
    database_path: &std::path::Path,
    now: DateTime<Utc>,
) -> Result<Option<LearningRun>, BridgeError> {
    if !database_path.is_file() {
        return Ok(None);
    }
    run_due(&open_existing_database(database_path)?, now)
}

fn parse_trigger(value: &str) -> Result<(LearningTriggerKind, Option<String>), BridgeError> {
    let trimmed = value.trim();
    if trimmed == "manual" {
        return Ok((LearningTriggerKind::Manual, None));
    }
    if trimmed == "in-app" || trimmed == "in_app" {
        return Ok((LearningTriggerKind::InApp, None));
    }
    for (prefix, kind) in [
        ("codex:", LearningTriggerKind::Codex),
        ("claude:", LearningTriggerKind::Claude),
        ("opencode:", LearningTriggerKind::OpenCode),
    ] {
        if let Some(registration_id) = trimmed.strip_prefix(prefix) {
            if registration_id.trim().is_empty() {
                break;
            }
            return Ok((kind, Some(registration_id.trim().to_owned())));
        }
    }
    Err(BridgeError::Invalid(
        "trigger must be manual, in-app, codex:<registration-id>, claude:<registration-id>, or opencode:<registration-id>"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::Effort,
        learning_router::{
            CandidateEvaluation, CandidateExclusion, CandidatePrediction, RouteCandidate,
            RouterDecision, RouterMode,
        },
        model::CapabilityTier,
        store,
    };

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/learning','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Pune','Learning','bridge/learning','/tmp/learning-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','idle','reported')", []).unwrap();
        db
    }

    fn candidate(model: &str) -> CandidateEvaluation {
        CandidateEvaluation {
            candidate: RouteCandidate {
                harness: "codex".into(),
                model: model.into(),
                tier: CapabilityTier::Standard,
                effort: Effort::Medium,
                sandbox: "workspace_write".into(),
                capabilities: vec!["tools".into(), "commands".into()],
                available: true,
                platform_supported: true,
                permission_eligible: true,
                quota_available: true,
                context_available: true,
                risk_eligible: true,
                capability_units: 2,
                default_for_tier: model == "a",
            },
            exclusions: vec![],
            prediction: CandidatePrediction {
                pass_probability_bps: 8_000,
                latency_ms: 100,
                normalized_quota_cost: 100,
                retry_risk_bps: 1_000,
                samples: 2,
            },
            expected_cost_score: 100,
        }
    }

    fn add_outcome(
        db: &Connection,
        index: i64,
        model: &str,
        success: bool,
        cost: Option<i64>,
        policy_version: i64,
    ) {
        let child = format!("child-{index}");
        let decision = format!("decision-{index}");
        let candidate_key = format!("codex:{model}");
        let body = RouterDecision {
            schema_version: crate::learning_router::ROUTER_SCHEMA_VERSION,
            id: decision.clone(),
            workspace_id: "w".into(),
            parent_session_id: "parent".into(),
            turn_id: format!("turn-{index}"),
            trace_id: Some("trace-learning".into()),
            task_family: "implementation".into(),
            task_fingerprint: "repeated-implementation".into(),
            repository_revision: Some("head:clean".into()),
            profile_version: Some(1),
            profile_purpose: Some("implementer".into()),
            policy_version,
            catalog_snapshot: json!({"codex":["a","b"]}),
            mode: if policy_version > 1 {
                RouterMode::Autonomous
            } else {
                RouterMode::Shadow
            },
            manual_override: false,
            baseline_candidate: Some(candidate_key.clone()),
            recommended_candidate: Some(candidate_key.clone()),
            executed_candidate: Some(candidate_key.clone()),
            explanation: "fixture".into(),
            candidates: vec![candidate("a"), candidate("b")],
            actual_provider: Some("codex".into()),
            actual_model: Some(model.into()),
            actual_effort: Some(Effort::Medium),
            created_at: "now".into(),
        };
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES(?1,'w','codex','Child','completed','reported','parent')", params![child]).unwrap();
        db.execute("INSERT INTO router_decisions(id,workspace_id,parent_session_id,turn_id,trace_id,task_family,task_fingerprint,profile_version,profile_purpose,policy_version,actual_provider,actual_model,actual_effort,mode,manual_override,baseline_candidate,recommended_candidate,executed_candidate,decision,created_at) VALUES(?1,'w','parent',?2,'trace-learning','implementation','repeated-implementation',1,'implementer',?3,'codex',?4,'medium',?5,0,?6,?6,?6,?7,'now')", params![decision, format!("turn-{index}"), policy_version, model, if policy_version > 1 { "autonomous" } else { "shadow" }, candidate_key, serde_json::to_string(&body).unwrap()]).unwrap();
        db.execute("INSERT INTO router_outcomes(decision_id,child_session_id,candidate,succeeded,status,runtime_ms,normalized_cost,retry_count,human_intervention,success_state,acceptance_state,cost_microusd,cost_source,confidence_bps,recorded_at) VALUES(?1,?2,?3,?4,?5,?6,1000,0,0,?7,?8,?9,?10,9000,'now')", params![decision, child, candidate_key, success, if success { "completed" } else { "failed" }, if model == "b" { 100 } else { 200 }, if success { "success" } else { "failure" }, if success { "accepted" } else { "rejected" }, cost, cost.map(|_| "provider_reported")]).unwrap();
    }

    fn add_improving_fixture(db: &Connection, policy_version: i64) {
        for (index, model, success) in [
            (0, "a", false),
            (1, "a", false),
            (2, "b", true),
            (3, "b", true),
            (4, "a", false),
            (5, "a", false),
            (6, "b", true),
            (7, "b", true),
            (8, "b", true),
            (9, "a", false),
        ] {
            add_outcome(
                db,
                index,
                model,
                success,
                Some(if model == "b" { 100 } else { 200 }),
                policy_version,
            );
        }
    }

    fn set_mode(db: &Connection, mode: &str) {
        let mut schedule = load_schedule(db).unwrap();
        schedule.mode = mode.into();
        update_schedule(db, &schedule).unwrap();
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
    fn one_job_lease_blocks_a_competing_newer_snapshot() {
        let db = database();
        db.execute(
            "INSERT INTO learning_job_runs(id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,lease_owner,lease_expires_at,snapshot_frozen_at,created_at)
             VALUES('active','default','manual','default:0:1',0,1,'running','owner',?1,'now','now')",
            params![(Utc::now() + Duration::minutes(5)).to_rfc3339()],
        )
        .unwrap();
        add_outcome(&db, 1, "a", false, Some(100), 1);
        let duplicate = run_learning(&db, LearningTriggerKind::Codex).unwrap();
        assert_eq!(duplicate.id, "active");
        assert!(duplicate.duplicate);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM learning_job_runs", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
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
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.status, LearningRunStatus::Completed, "{:?}", run.report);
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
            run_budget_tokens: 10_000,
            mode: "manual".into(),
        };
        update_schedule(&db, &schedule).unwrap();
        db.execute("UPDATE sessions SET status='working' WHERE id='parent'", [])
            .unwrap();
        assert!(run_due(&db, now).unwrap().is_none());
        db.execute("UPDATE sessions SET status='idle' WHERE id='parent'", [])
            .unwrap();
        assert!(run_due(&db, now).unwrap().is_some());
        assert!(run_due(&db, now).unwrap().is_none());
        let next = DateTime::parse_from_rfc3339(
            load_schedule(&db).unwrap().next_run_at.as_deref().unwrap(),
        )
        .unwrap();
        assert!(next > now);
    }

    #[test]
    fn subsequent_runs_require_an_incremental_evidence_batch() {
        let db = database();
        add_improving_fixture(&db, 1);
        assert_eq!(
            run_learning(&db, LearningTriggerKind::Manual)
                .unwrap()
                .status,
            LearningRunStatus::Completed
        );
        add_outcome(&db, 100, "b", true, Some(100), 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.status, LearningRunStatus::Noop);
        assert!(run
            .report
            .unwrap()
            .reason
            .contains("insufficient new evidence: 1/5"));
        for index in 101..105 {
            add_outcome(&db, index, "b", true, Some(100), 1);
        }
        let accumulated = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(accumulated.status, LearningRunStatus::Completed);
        assert_eq!(
            db.query_row(
                "SELECT last_evidence_boundary FROM learning_jobs WHERE id='default'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            accumulated.evidence_boundary
        );
    }

    #[test]
    fn external_trigger_parser_requires_narrow_registration_ids() {
        assert_eq!(
            parse_trigger("codex:daily-learning").unwrap(),
            (LearningTriggerKind::Codex, Some("daily-learning".into()))
        );
        assert_eq!(
            parse_trigger("claude:desktop-task").unwrap(),
            (LearningTriggerKind::Claude, Some("desktop-task".into()))
        );
        assert!(parse_trigger("codex:").is_err());
        assert!(parse_trigger("provider:free-form").is_err());
        let prompt = trigger_instructions(
            LearningTriggerKind::Codex,
            "/tmp/bridge.db",
            "daily-learning",
        )
        .unwrap();
        assert!(prompt.contains(
            "bridge learning run --database \"/tmp/bridge.db\" --trigger codex:daily-learning"
        ));
        assert!(prompt.contains("wake-up trigger only"));
        assert!(!prompt.contains("{{"));
    }

    #[test]
    fn external_trigger_does_not_run_desktop_recovery_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        {
            let db = store::open(&path).unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/external-learning','now')", []).unwrap();
            db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Pune','Learning','bridge/learning','/tmp/external-learning-w','working','now')", []).unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('active','w','codex','Active','working','reported')", []).unwrap();
            register_trigger(&db, LearningTriggerKind::Codex, "scheduled", None).unwrap();
        }
        assert!(
            run_database(&path, "codex:scheduled", None)
                .unwrap()
                .accepted
        );
        let db = Connection::open(path).unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM sessions WHERE id='active'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "working"
        );
    }

    #[test]
    fn locked_database_fails_without_partial_external_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut owner = store::open(&path).unwrap();
        register_trigger(&owner, LearningTriggerKind::Codex, "locked", None).unwrap();
        let lock = owner.transaction().unwrap();
        lock.execute(
            "UPDATE learning_jobs SET updated_at='locked' WHERE id='default'",
            [],
        )
        .unwrap();
        let contender = Connection::open(&path).unwrap();
        contender
            .busy_timeout(std::time::Duration::from_millis(5))
            .unwrap();
        assert!(
            run_external_trigger(&contender, LearningTriggerKind::Codex, "locked", None).is_err()
        );
        lock.rollback().unwrap();
        assert_eq!(
            owner
                .query_row("SELECT COUNT(*) FROM learning_job_runs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn cost_per_success_requires_complete_provider_costs() {
        let complete = database();
        add_improving_fixture(&complete, 1);
        let report = run_learning(&complete, LearningTriggerKind::Manual)
            .unwrap()
            .report
            .unwrap();
        assert!(report.cost_complete);
        assert_eq!(report.average_cost_microusd, Some(300));

        let incomplete = database();
        add_improving_fixture(&incomplete, 1);
        incomplete
            .execute(
                "UPDATE router_outcomes SET cost_microusd=NULL,cost_source=NULL WHERE rowid=1",
                [],
            )
            .unwrap();
        let report = run_learning(&incomplete, LearningTriggerKind::Manual)
            .unwrap()
            .report
            .unwrap();
        assert!(!report.cost_complete);
        assert_eq!(report.average_cost_microusd, None);
    }

    #[test]
    fn lease_expiry_and_trigger_auth_are_auditable() {
        let db = database();
        let now = Utc::now();
        db.execute(
            "INSERT INTO learning_job_runs(id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,lease_owner,lease_expires_at,snapshot_frozen_at,created_at)
             VALUES('stale','default','manual','default:0:1',0,1,'running','old-owner',?1,?2,?2)",
            params![(now - Duration::minutes(1)).to_rfc3339(), (now - Duration::minutes(20)).to_rfc3339()],
        )
        .unwrap();
        add_outcome(&db, 99, "a", false, Some(100), 1);
        let recovered = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(recovered.id, "stale");
        assert_eq!(recovered.evidence_boundary, 0);
        assert_eq!(recovered.status, LearningRunStatus::Noop);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM learning_trigger_events WHERE result='lease_recovered'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );

        let auth = database();
        let missing =
            run_external_trigger(&auth, LearningTriggerKind::Codex, "missing", None).unwrap();
        assert!(!missing.accepted);
        register_trigger_with_expiry(
            &auth,
            LearningTriggerKind::Codex,
            "expired",
            None,
            Some(&(Utc::now() - Duration::minutes(1)).to_rfc3339()),
        )
        .unwrap();
        assert!(
            !run_external_trigger(&auth, LearningTriggerKind::Codex, "expired", None)
                .unwrap()
                .accepted
        );

        let credentialed = database();
        register_trigger(
            &credentialed,
            LearningTriggerKind::Codex,
            "daily",
            Some("keychain:bridge/codex-daily"),
        )
        .unwrap();
        assert!(
            !run_external_trigger(
                &credentialed,
                LearningTriggerKind::Codex,
                "daily",
                Some("keychain:bridge/wrong"),
            )
            .unwrap()
            .accepted
        );
        assert!(
            run_external_trigger(
                &credentialed,
                LearningTriggerKind::Codex,
                "daily",
                Some("keychain:bridge/codex-daily"),
            )
            .unwrap()
            .accepted
        );
        assert_eq!(
            credentialed.query_row(
                "SELECT COUNT(*) FROM session_entries WHERE payload LIKE '%keychain:bridge/codex-daily%'",
                [],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            0
        );

        register_trigger(&credentialed, LearningTriggerKind::Claude, "desktop", None).unwrap();
        let enabled: (bool, bool) = credentialed
            .query_row(
                "SELECT
                    EXISTS(SELECT 1 FROM learning_triggers WHERE kind='codex' AND enabled=1),
                    EXISTS(SELECT 1 FROM learning_triggers WHERE kind='claude' AND enabled=1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(enabled, (true, false));
        enable_trigger(&credentialed, LearningTriggerKind::Claude, "desktop").unwrap();
        let enabled: (bool, bool) = credentialed.query_row(
            "SELECT EXISTS(SELECT 1 FROM learning_triggers WHERE kind='codex' AND enabled=1),EXISTS(SELECT 1 FROM learning_triggers WHERE kind='claude' AND enabled=1)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(enabled, (false, true));
    }

    #[test]
    fn ask_requires_approval_and_promotion_rollback_are_versioned() {
        let db = database();
        set_mode(&db, "ask");
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.promotion_status, "awaiting_approval");
        assert_eq!(active_policy_version(&db).unwrap(), 1);
        let approved = approve_run(&db, &run.id).unwrap();
        assert_eq!(approved.promotion_status, "promoted");
        assert_eq!(active_policy_version(&db).unwrap(), 2);
        let rollback = rollback_policy(&db, 1, "fixture regression").unwrap();
        assert_eq!(rollback, 3);
        assert_eq!(active_policy_version(&db).unwrap(), 3);
        assert_eq!(
            db.query_row(
                "SELECT predecessor,rollback_of FROM routing_policies WHERE version=3",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            (2, 1)
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='active'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn cancellation_before_ask_promotion_abandons_candidate() {
        let db = database();
        set_mode(&db, "ask");
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        let cancelled = cancel_run(&db, &run.id).unwrap();
        assert_eq!(cancelled.status, LearningRunStatus::Cancelled);
        assert!(approve_run(&db, &run.id).is_err());
        assert_eq!(active_policy_version(&db).unwrap(), 1);
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "abandoned"
        );
    }

    #[test]
    fn automatic_canary_rolls_back_on_regression() {
        let db = database();
        set_mode(&db, "automatic");
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.promotion_status, "canary");
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "canary"
        );
        for index in 100..105 {
            add_outcome(&db, index, "b", false, Some(2_000), 2);
        }
        settle_canary(&db).unwrap();
        assert_eq!(active_policy_version(&db).unwrap(), 3);
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "rolled_back"
        );
        assert_eq!(
            db.query_row(
                "SELECT rollback_of FROM routing_policies WHERE version=3",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn automatic_canary_becomes_active_only_after_green_outcomes() {
        let db = database();
        set_mode(&db, "automatic");
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.promotion_status, "canary");
        for index in 100..105 {
            add_outcome(&db, index, "b", true, Some(100), 2);
        }
        settle_canary(&db).unwrap();
        assert_eq!(active_policy_version(&db).unwrap(), 2);
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policy_promotions WHERE action='canary_completed'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn automatic_canary_holds_when_required_cost_is_unknown() {
        let db = database();
        set_mode(&db, "automatic");
        add_improving_fixture(&db, 1);
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(run.promotion_status, "canary");
        for index in 100..105 {
            add_outcome(&db, index, "b", true, None, 2);
        }
        settle_canary(&db).unwrap();
        assert_eq!(active_policy_version(&db).unwrap(), 2);
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "canary"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE rollback_of IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn failed_run_abandons_its_linked_candidate_atomically() {
        let db = database();
        db.execute(
            "INSERT INTO routing_policies(version,status,predecessor,weights,thresholds,created_reason,created_at)
             VALUES(2,'candidate',1,'{}','{}','fixture','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO learning_job_runs(id,job_id,trigger_kind,idempotency_key,evidence_boundary,base_policy_version,status,snapshot_frozen_at,candidate_policy_version,created_at)
             VALUES('failed-fixture','default','manual','default:0:1',0,1,'running','now',2,'now')",
            [],
        )
        .unwrap();
        let failed = fail_run(
            &db,
            "failed-fixture",
            &BridgeError::Invalid("injected failure".into()),
        )
        .unwrap();
        assert_eq!(failed.status, LearningRunStatus::Failed);
        assert_eq!(
            db.query_row(
                "SELECT status FROM routing_policies WHERE version=2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "abandoned"
        );
    }

    #[test]
    fn automatic_mode_never_stacks_canary_promotions() {
        let db = database();
        set_mode(&db, "automatic");
        add_improving_fixture(&db, 1);
        let first = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(first.promotion_status, "canary");
        add_outcome(&db, 100, "b", true, Some(100), 2);
        let second = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        assert_eq!(second.status, LearningRunStatus::Noop);
        assert!(second
            .report
            .unwrap()
            .reason
            .contains("still collecting outcomes"));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='canary'",
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
            0
        );
    }

    #[test]
    fn replay_preserves_observed_rates_instead_of_synthetic_majorities() {
        let db = database();
        add_improving_fixture(&db, 1);
        db.execute(
            "UPDATE router_outcomes SET succeeded=0,status='failed',success_state='failure',acceptance_state='rejected' WHERE decision_id='decision-8'",
            [],
        )
        .unwrap();
        let candidate = routing_policy::build_candidate(&db, 10, &json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(candidate.replay.candidate.quality_bps, Some(8_000));
        assert!(candidate.replay.quality_guard_passed);
    }

    #[test]
    fn replay_fixtures_cover_cost_regression_and_unavailable_models() {
        let cost_db = database();
        for (index, model, success, cost) in [
            (0, "a", false, 100),
            (1, "a", false, 100),
            (2, "b", true, 1_000),
            (3, "b", true, 1_000),
            (4, "a", true, 100),
            (5, "a", false, 100),
            (6, "b", true, 1_000),
            (7, "b", true, 1_000),
            (8, "b", true, 1_000),
            (9, "a", true, 100),
        ] {
            add_outcome(&cost_db, index, model, success, Some(cost), 1);
        }
        let cost_candidate = routing_policy::build_candidate(&cost_db, 10, &json!({}))
            .unwrap()
            .unwrap();
        assert!(!cost_candidate.replay.cost_guard_passed);
        assert!(!cost_candidate.replay.passed);

        let unavailable_db = database();
        add_improving_fixture(&unavailable_db, 1);
        for decision_id in ["decision-4", "decision-9"] {
            let body: String = unavailable_db
                .query_row(
                    "SELECT decision FROM router_decisions WHERE id=?1",
                    params![decision_id],
                    |row| row.get(0),
                )
                .unwrap();
            let mut decision: RouterDecision = serde_json::from_str(&body).unwrap();
            decision.candidates[1]
                .exclusions
                .push(CandidateExclusion::HarnessUnavailable);
            unavailable_db
                .execute(
                    "UPDATE router_decisions SET decision=?2 WHERE id=?1",
                    params![decision_id, serde_json::to_string(&decision).unwrap()],
                )
                .unwrap();
        }
        let unavailable = routing_policy::build_candidate(&unavailable_db, 10, &json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(unavailable.replay.unavailable_selections, 2);
        assert!(!unavailable.replay.passed);
    }

    #[test]
    fn insufficient_deterministic_evidence_queues_only_bounded_independent_eval() {
        let db = database();
        add_improving_fixture(&db, 1);
        db.execute(
            "UPDATE router_outcomes SET success_state='unknown',confidence_bps=4000 WHERE rowid=1",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO model_setup_state(id,active_version,updated_at) VALUES('default',1,'now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO model_profiles(version,purpose,canonical_role,provider,model,effort,pinned,learning_enabled,created_at)
             VALUES(1,'evaluator','verification','claude','independent-evaluator','high',0,1,'now')",
            [],
        ).unwrap();
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        let evaluation: (String, String, String) = db.query_row(
            "SELECT evaluator_version,status,bounded_metrics FROM routing_evaluations WHERE learning_run_id=?1 AND evaluator_kind='model_based'",
            params![run.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert!(evaluation.0.contains("claude:independent-evaluator"));
        assert_eq!(evaluation.1, "pending_bounded_model_eval");
        assert!(evaluation.2.contains("\"toolAccess\":\"none\""));
        assert!(evaluation.2.contains("\"transcriptIncluded\":false"));
        assert!(!evaluation.2.contains("fixture"));
    }

    #[test]
    fn typed_independent_verifier_closes_model_eval_without_transcript_or_tools() {
        let db = database();
        add_improving_fixture(&db, 1);
        db.execute(
            "UPDATE router_outcomes SET success_state='unknown',confidence_bps=4000 WHERE rowid=1",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_committed,status,created_at,updated_at) VALUES('contract','w','parent',1,'[]',0,'verified','now','now')", []).unwrap();
        db.execute("INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES('plan','contract',1,'high','{}','now')", []).unwrap();
        db.execute("INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,implementer_family,started_at,completed_at) VALUES('attempt','plan','parent','head','clean','/tmp','verified','codex','now','now')", []).unwrap();
        db.execute("INSERT INTO eval_check_runs(id,attempt_id,check_id,kind,required,status,executor,verifier_family,output_digest,artifact_refs,started_at,completed_at) VALUES('check','attempt','scrutiny','scrutiny',1,'passed','bridge.worker_result','claude','abc123','[\"proof:check\"]','now','now')", []).unwrap();
        let run = run_learning(&db, LearningTriggerKind::Manual).unwrap();
        let evaluation: (String, String, Option<i64>, Option<i64>, String, String) = db.query_row(
            "SELECT evaluator_version,status,score_bps,confidence_bps,evidence_entry_ids,bounded_metrics FROM routing_evaluations WHERE learning_run_id=?1 AND evaluator_kind='model_based'",
            params![run.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        ).unwrap();
        assert_eq!(evaluation.0, "independent:claude:completion-v1");
        assert_eq!(evaluation.1, "completed");
        assert_eq!(evaluation.2, Some(10_000));
        assert_eq!(evaluation.3, Some(9_000));
        assert!(evaluation.4.contains("proof:check"));
        assert!(evaluation.4.contains("digest:abc123"));
        assert!(evaluation.5.contains("\"toolAccess\":\"none\""));
        assert!(!evaluation.5.contains("detail"));
    }
}
