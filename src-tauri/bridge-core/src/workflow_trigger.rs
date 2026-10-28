//! One claim path for every workflow trigger.
//!
//! Run now, a schedule slot and a catch-up all arrive at [`claim`], and no run
//! row is inserted any other way. That single entrance is what makes the
//! guarantees checkable:
//!
//! * **One run per occasion.** Every occasion has an idempotency key; a second
//!   claim for it gets the first run back.
//! * **One active run per workflow.** A partial unique index forbids two, and a
//!   schedule slot that arrives while one is active is recorded as `skipped`.
//! * **A lease that outlives a crash.** A claimed run holds a 15-minute lease,
//!   heartbeated by its runner and reclaimed by compare-and-swap once it
//!   expires. A worker that lost its lease cannot write a result.
//! * **One catch-up, never a burst.** After a sleep, only the latest missed slot
//!   runs, labelled with how many it stands for.
//!
//! The lease shape is `work_briefing_trigger.rs`'s, deliberately.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use bridge_protocol::messages as wire;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::workflow_schedule::Schedule;
use crate::workflows;
use crate::BridgeError;

/// How long a claim holds without a heartbeat.
pub const LEASE_MINUTES: i64 = 15;
/// A slot claimed later than this after its time counts as a catch-up.
pub const ON_TIME_MINUTES: i64 = 5;
/// How often the maintenance loop looks for due slots and expired leases.
pub const TICK_SECONDS: u64 = 30;
pub const DEFAULT_QUOTA_RESERVE_PERCENT: i64 = 25;
pub const MAX_QUOTA_RESERVE_PERCENT: i64 = 90;

const SETTINGS_KIND: &str = "workflows";
const SETTINGS_ID: &str = "settings";
const ACTIVE: &str = "('queued','waiting_quota','running','waiting_approval')";

/// Every timestamp this module writes. One format, so string order is time
/// order within these columns.
pub fn stamp(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|instant| instant.with_timezone(&Utc))
}

fn enum_text<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => unreachable!("workflow enums serialize as strings"),
    }
}

fn enum_from<T: for<'de> Deserialize<'de>>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text.clone())).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, format!("unknown value `{text}`").into())
    })
}

// --- settings --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    paused: bool,
    quota_reserve_percent: i64,
    /// When Workflows were last unpaused: a schedule anchor, so the paused
    /// stretch is never caught up.
    #[serde(default)]
    resumed_at: Option<String>,
}

fn stored_settings(db: &Connection) -> Result<Option<StoredSettings>, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(format!("stored workflow settings are unreadable: {error}"))))
        .transpose()
}

pub fn default_settings() -> wire::WorkflowSettings {
    wire::WorkflowSettings { paused: false, quota_reserve_percent: DEFAULT_QUOTA_RESERVE_PERCENT }
}

pub fn read_settings(db: &Connection) -> Result<wire::WorkflowSettingsSnapshot, BridgeError> {
    Ok(match stored_settings(db)? {
        Some(stored) => wire::WorkflowSettingsSnapshot {
            configured: true,
            settings: wire::WorkflowSettings { paused: stored.paused, quota_reserve_percent: stored.quota_reserve_percent },
        },
        None => wire::WorkflowSettingsSnapshot { configured: false, settings: default_settings() },
    })
}

pub fn write_settings(
    db: &Connection,
    settings: wire::WorkflowSettings,
    now: DateTime<Utc>,
) -> Result<wire::WorkflowSettingsSnapshot, BridgeError> {
    if !(0..=MAX_QUOTA_RESERVE_PERCENT).contains(&settings.quota_reserve_percent) {
        return Err(BridgeError::Invalid(format!(
            "The quota reserve is between 0 and {MAX_QUOTA_RESERVE_PERCENT} percent"
        )));
    }
    let previous = stored_settings(db)?;
    let was_paused = previous.as_ref().is_some_and(|stored| stored.paused);
    let resumed_at = if was_paused && !settings.paused {
        Some(stamp(now))
    } else {
        previous.and_then(|stored| stored.resumed_at)
    };
    let stored = StoredSettings { paused: settings.paused, quota_reserve_percent: settings.quota_reserve_percent, resumed_at };
    let payload = serde_json::to_string(&stored).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at",
        params![SETTINGS_KIND, SETTINGS_ID, payload, stamp(now)],
    )?;
    read_settings(db)
}

fn paused(db: &Connection) -> Result<bool, BridgeError> {
    Ok(stored_settings(db)?.is_some_and(|stored| stored.paused))
}

// --- runs ------------------------------------------------------------------------

const SELECT_RUN: &str = "SELECT id, workflow_id, revision_id, trigger_kind, missed_slots, dry_run, status,
        failure_code, summary, session_id, quota_unknown, cost_microusd, tokens, unread, scheduled_for,
        started_at, finished_at, created_at
     FROM workflow_runs";

fn read_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<wire::WorkflowRun> {
    Ok(wire::WorkflowRun {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        revision_id: row.get(2)?,
        trigger_kind: enum_from(row.get(3)?)?,
        missed_slots: row.get(4)?,
        dry_run: row.get(5)?,
        status: enum_from(row.get(6)?)?,
        failure_code: row.get(7)?,
        summary: row.get(8)?,
        session_id: row.get(9)?,
        quota_unknown: row.get(10)?,
        cost_microusd: row.get(11)?,
        tokens: row.get(12)?,
        unread: row.get(13)?,
        scheduled_for: row.get(14)?,
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
        created_at: row.get(17)?,
    })
}

pub fn get_run(db: &Connection, run_id: &str) -> Result<Option<wire::WorkflowRun>, BridgeError> {
    Ok(db.query_row(&format!("{SELECT_RUN} WHERE id=?1"), params![run_id], read_run).optional()?)
}

pub fn require_run(db: &Connection, run_id: &str) -> Result<wire::WorkflowRun, BridgeError> {
    get_run(db, run_id)?.ok_or_else(|| BridgeError::Invalid(format!("workflow run {run_id} does not exist")))
}

/// The newest run of a workflow, skipped runs included.
pub fn latest_run(db: &Connection, workflow_id: &str) -> Result<Option<wire::WorkflowRun>, BridgeError> {
    Ok(db
        .query_row(
            &format!("{SELECT_RUN} WHERE workflow_id=?1 ORDER BY created_at DESC, rowid DESC LIMIT 1"),
            params![workflow_id],
            read_run,
        )
        .optional()?)
}

fn active_run(db: &Connection, workflow_id: &str) -> Result<Option<wire::WorkflowRun>, BridgeError> {
    Ok(db
        .query_row(&format!("{SELECT_RUN} WHERE workflow_id=?1 AND status IN {ACTIVE}"), params![workflow_id], read_run)
        .optional()?)
}

/// Runs newest first, optionally for one workflow or unread only.
pub fn list_runs(
    db: &Connection,
    workflow_id: Option<&str>,
    unread_only: bool,
    limit: i64,
) -> Result<Vec<wire::WorkflowRun>, BridgeError> {
    if !(1..=200).contains(&limit) {
        return Err(BridgeError::Invalid("List between 1 and 200 runs".into()));
    }
    let mut statement = db.prepare(&format!(
        "{SELECT_RUN} WHERE (?1 IS NULL OR workflow_id=?1) AND (?2=0 OR unread=1)
         ORDER BY created_at DESC, rowid DESC LIMIT ?3"
    ))?;
    let runs = statement
        .query_map(params![workflow_id, unread_only, limit], read_run)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(runs)
}

// --- claim -----------------------------------------------------------------------

/// What a trigger is asking to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occasion {
    pub kind: wire::WorkflowTriggerKind,
    /// `manual:<workflow>:<request>` or `schedule:<workflow>:<slot>`.
    pub key: String,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub missed_slots: i64,
}

impl Occasion {
    pub fn manual(workflow_id: &str, request_id: &str) -> Self {
        Self {
            kind: wire::WorkflowTriggerKind::Manual,
            key: format!("manual:{workflow_id}:{request_id}"),
            scheduled_for: None,
            missed_slots: 0,
        }
    }

    /// A schedule slot. Schedule and catch-up share one key per slot, so a slot
    /// is never run twice under two names.
    pub fn slot(workflow_id: &str, slot: DateTime<Utc>, kind: wire::WorkflowTriggerKind, missed_slots: i64) -> Self {
        Self { kind, key: format!("schedule:{workflow_id}:{}", stamp(slot)), scheduled_for: Some(slot), missed_slots }
    }
}

/// A run this claim now owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRun {
    pub run: wire::WorkflowRun,
    /// The token every later write must present.
    pub lease_owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed(ClaimedRun),
    /// This occasion already has a run.
    Existing(wire::WorkflowRun),
    /// Another run of the workflow is active. A schedule slot is recorded as
    /// `skipped`; Run now just gets the active run back.
    Overlap { active: wire::WorkflowRun, skipped: Option<wire::WorkflowRun> },
    Refused(String),
}

fn immediate(db: &Connection) -> Result<Transaction<'_>, BridgeError> {
    Ok(Transaction::new_unchecked(db, TransactionBehavior::Immediate)?)
}

/// The one entrance. See the module docs.
pub fn claim(
    db: &Connection,
    workflow_id: &str,
    occasion: &Occasion,
    dry_run: bool,
    now: DateTime<Utc>,
) -> Result<ClaimOutcome, BridgeError> {
    let transaction = immediate(db)?;
    let outcome = claim_in(&transaction, workflow_id, occasion, dry_run, now)?;
    transaction.commit()?;
    Ok(outcome)
}

fn claim_in(
    db: &Connection,
    workflow_id: &str,
    occasion: &Occasion,
    dry_run: bool,
    now: DateTime<Utc>,
) -> Result<ClaimOutcome, BridgeError> {
    if let Some(run) = db
        .query_row(&format!("{SELECT_RUN} WHERE idempotency_key=?1"), params![occasion.key], read_run)
        .optional()?
    {
        return Ok(ClaimOutcome::Existing(run));
    }
    workflows::reconcile_projects(db)?;
    let Some(workflow) = workflows::get(db, workflow_id)? else {
        return Ok(ClaimOutcome::Refused(format!("workflow {workflow_id} does not exist")));
    };
    if !workflow.enabled {
        let reason = match workflow.disabled_reason.as_deref() {
            Some(workflows::DISABLED_PROJECT_REMOVED) => "its project is gone",
            _ => "it is turned off",
        };
        return Ok(ClaimOutcome::Refused(format!("{} cannot run: {reason}", workflow.definition.name)));
    }
    let manual = occasion.kind == wire::WorkflowTriggerKind::Manual;
    // A person may dry-run what they have not approved yet: every write is a
    // preview. Nothing else runs unapproved.
    if !workflow.approved && !(manual && dry_run) {
        return Ok(ClaimOutcome::Refused(format!("{} waits for approval", workflow.definition.name)));
    }
    if paused(db)? {
        return Ok(ClaimOutcome::Refused("Workflows are paused".into()));
    }

    if let Some(active) = active_run(db, workflow_id)? {
        if manual {
            return Ok(ClaimOutcome::Overlap { active, skipped: None });
        }
        let id = Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO workflow_runs(id,workflow_id,revision_id,trigger_kind,occasion,idempotency_key,missed_slots,
                dry_run,status,failure_code,scheduled_for,finished_at,created_at)
             VALUES(?1,?2,?3,?4,?5,?5,?6,0,'skipped','overlap',?7,?8,?8)",
            params![
                id,
                workflow_id,
                workflow.revision_id,
                enum_text(occasion.kind),
                occasion.key,
                occasion.missed_slots,
                occasion.scheduled_for.map(stamp),
                stamp(now),
            ],
        )?;
        return Ok(ClaimOutcome::Overlap { active, skipped: Some(require_run(db, &id)?) });
    }

    let id = Uuid::new_v4().to_string();
    let lease_owner = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO workflow_runs(id,workflow_id,revision_id,trigger_kind,occasion,idempotency_key,missed_slots,
            dry_run,status,lease_owner,lease_expires_at,scheduled_for,created_at)
         VALUES(?1,?2,?3,?4,?5,?5,?6,?7,'queued',?8,?9,?10,?11)",
        params![
            id,
            workflow_id,
            workflow.revision_id,
            enum_text(occasion.kind),
            occasion.key,
            occasion.missed_slots,
            dry_run,
            lease_owner,
            stamp(now + Duration::minutes(LEASE_MINUTES)),
            occasion.scheduled_for.map(stamp),
            stamp(now),
        ],
    )?;
    Ok(ClaimOutcome::Claimed(ClaimedRun { run: require_run(db, &id)?, lease_owner }))
}

// --- lease ------------------------------------------------------------------------

/// Extend the lease. False means the run was reclaimed or cancelled: the
/// caller must stop and write nothing.
pub fn heartbeat(db: &Connection, run_id: &str, lease_owner: &str, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    let changed = db.execute(
        &format!(
            "UPDATE workflow_runs SET lease_expires_at=?3
             WHERE id=?1 AND lease_owner=?2 AND status IN {ACTIVE} AND lease_expires_at > ?4"
        ),
        params![run_id, lease_owner, stamp(now + Duration::minutes(LEASE_MINUTES)), stamp(now)],
    )?;
    Ok(changed == 1)
}

/// Move a held run to another active status. False means the lease was lost.
pub fn transition(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    status: wire::WorkflowRunStatus,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    if !status.is_active() {
        return Err(BridgeError::Invalid("transition only moves between active statuses; use finish".into()));
    }
    let changed = db.execute(
        &format!(
            "UPDATE workflow_runs SET status=?3, session_id=COALESCE(?4, session_id),
                started_at=CASE WHEN ?3='running' THEN COALESCE(started_at, ?5) ELSE started_at END
             WHERE id=?1 AND lease_owner=?2 AND status IN {ACTIVE} AND lease_expires_at > ?5"
        ),
        params![run_id, lease_owner, enum_text(status), session_id, stamp(now)],
    )?;
    Ok(changed == 1)
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub failure_code: Option<String>,
    pub summary: Option<String>,
    pub cost_microusd: Option<i64>,
    pub tokens: Option<i64>,
    pub quota_unknown: bool,
}

/// Write a terminal status. Only the holder of an unexpired lease may; false
/// means it was lost and nothing was written.
pub fn finish(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    status: wire::WorkflowRunStatus,
    outcome: &Outcome,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    if status.is_active() || status == wire::WorkflowRunStatus::Skipped {
        return Err(BridgeError::Invalid("finish writes a terminal status".into()));
    }
    let changed = db.execute(
        &format!(
            "UPDATE workflow_runs SET status=?3, failure_code=?4, summary=?5, cost_microusd=?6, tokens=?7,
                quota_unknown=?8, lease_owner=NULL, lease_expires_at=NULL, finished_at=?9, unread=1
             WHERE id=?1 AND lease_owner=?2 AND status IN {ACTIVE} AND lease_expires_at > ?9"
        ),
        params![
            run_id,
            lease_owner,
            enum_text(status),
            outcome.failure_code,
            outcome.summary,
            outcome.cost_microusd,
            outcome.tokens,
            outcome.quota_unknown,
            stamp(now),
        ],
    )?;
    Ok(changed == 1)
}

/// Settle every active run whose lease expired. One compare-and-swap per row:
/// zero rows changed means its holder heartbeated first, and it is left alone.
pub fn reclaim_expired(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT id, lease_expires_at FROM workflow_runs
         WHERE status IN {ACTIVE} AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?1"
    ))?;
    let expired = statement
        .query_map(params![stamp(now)], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut reclaimed = Vec::new();
    for (id, expiry) in expired {
        let changed = db.execute(
            &format!(
                "UPDATE workflow_runs SET status='failed', failure_code='lease_expired', lease_owner=NULL,
                    lease_expires_at=NULL, finished_at=?3, unread=1
                 WHERE id=?1 AND lease_expires_at=?2 AND status IN {ACTIVE}"
            ),
            params![id, expiry, stamp(now)],
        )?;
        if changed == 1 {
            reclaimed.push(id);
        }
    }
    Ok(reclaimed)
}

/// Park a held run until its provider has room (C11). It stays active, so
/// nothing overlaps it, but holds no lease: nobody is working on it.
pub fn park(db: &Connection, run_id: &str, lease_owner: &str) -> Result<bool, BridgeError> {
    let changed = db.execute(
        &format!(
            "UPDATE workflow_runs SET status='waiting_quota', lease_owner=NULL, lease_expires_at=NULL
             WHERE id=?1 AND lease_owner=?2 AND status IN {ACTIVE}"
        ),
        params![run_id, lease_owner],
    )?;
    Ok(changed == 1)
}

/// Take every parked run back with a fresh lease, for the runner to re-check.
pub fn unpark_waiting(db: &Connection, now: DateTime<Utc>) -> Result<Vec<ClaimedRun>, BridgeError> {
    let ids: Vec<String> = db
        .prepare("SELECT id FROM workflow_runs WHERE status='waiting_quota' AND lease_owner IS NULL ORDER BY created_at")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut claimed = Vec::new();
    for id in ids {
        let lease_owner = Uuid::new_v4().to_string();
        let changed = db.execute(
            "UPDATE workflow_runs SET status='queued', lease_owner=?2, lease_expires_at=?3
             WHERE id=?1 AND status='waiting_quota' AND lease_owner IS NULL",
            params![id, lease_owner, stamp(now + Duration::minutes(LEASE_MINUTES))],
        )?;
        if changed == 1 {
            claimed.push(ClaimedRun { run: require_run(db, &id)?, lease_owner });
        }
    }
    Ok(claimed)
}

/// Cancel an active run. A queued or waiting run settles now; a running run's
/// holder learns on its next heartbeat, which fails because the lease is gone.
pub fn cancel(db: &Connection, run_id: &str, now: DateTime<Utc>) -> Result<wire::WorkflowRun, BridgeError> {
    let changed = db.execute(
        &format!(
            "UPDATE workflow_runs SET status='cancelled', lease_owner=NULL, lease_expires_at=NULL, finished_at=?2
             WHERE id=?1 AND status IN {ACTIVE}"
        ),
        params![run_id, stamp(now)],
    )?;
    let run = require_run(db, run_id)?;
    if changed == 0 {
        return Err(BridgeError::Invalid(format!("workflow run {run_id} already finished")));
    }
    Ok(run)
}

// --- schedule ---------------------------------------------------------------------

/// The latest instant a schedule counts slots from (F13a).
fn anchor(db: &Connection, workflow: &wire::Workflow) -> Result<DateTime<Utc>, BridgeError> {
    let last_slot: Option<String> = db.query_row(
        "SELECT MAX(scheduled_for) FROM workflow_runs WHERE workflow_id=?1 AND scheduled_for IS NOT NULL",
        params![workflow.id],
        |row| row.get(0),
    )?;
    let resumed = stored_settings(db)?.and_then(|stored| stored.resumed_at);
    Ok([last_slot.as_deref(), workflow.approved_at.as_deref(), Some(workflow.updated_at.as_str()), resumed.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(parse_instant)
        .max()
        .unwrap_or(DateTime::<Utc>::MIN_UTC))
}

fn schedule_of(workflow: &wire::Workflow) -> Option<Schedule> {
    match &workflow.definition.trigger {
        wire::WorkflowTrigger::Schedule { cron, timezone } => Schedule::parse(cron, timezone).ok(),
        wire::WorkflowTrigger::Manual => None,
    }
}

fn schedulable(workflow: &wire::Workflow) -> bool {
    workflow.enabled && workflow.approved
}

/// At most one occasion per scheduled workflow: the latest slot since its
/// anchor, as a schedule run when on time, else as a catch-up (C5).
pub fn due_occasions(db: &Connection, now: DateTime<Utc>) -> Result<Vec<(String, Occasion)>, BridgeError> {
    if paused(db)? {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = db
        .prepare("SELECT id FROM workflows WHERE enabled=1 ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut due = Vec::new();
    for id in ids {
        let Some(workflow) = workflows::get(db, &id)? else { continue };
        if !schedulable(&workflow) {
            continue;
        }
        let Some(schedule) = schedule_of(&workflow) else { continue };
        let mut cursor = anchor(db, &workflow)?;
        let (mut count, mut latest) = (0_i64, None);
        loop {
            let batch = schedule.fires_between(cursor, now, 1000);
            let Some(last) = batch.last().copied() else { break };
            count += batch.len() as i64;
            latest = Some(last);
            cursor = last;
            if batch.len() < 1000 {
                break;
            }
        }
        let Some(slot) = latest else { continue };
        let on_time = now - slot <= Duration::minutes(ON_TIME_MINUTES);
        let (kind, missed) = if on_time {
            (wire::WorkflowTriggerKind::Schedule, count - 1)
        } else {
            (wire::WorkflowTriggerKind::CatchUp, count)
        };
        due.push((id.clone(), Occasion::slot(&id, slot, kind, missed)));
    }
    Ok(due)
}

/// When the workflow will next fire on its schedule, if it can.
pub fn next_run_at(db: &Connection, workflow: &wire::Workflow, now: DateTime<Utc>) -> Result<Option<String>, BridgeError> {
    if !schedulable(workflow) || paused(db)? {
        return Ok(None);
    }
    Ok(schedule_of(workflow).and_then(|schedule| schedule.next_after(now)).map(stamp))
}

// --- maintenance ---------------------------------------------------------------------

fn publish_run(core: &Arc<BridgeCore>, run: wire::WorkflowRun) {
    core.events.publish(CoreEvent::WorkflowRunChanged { run });
}

/// Publish the current state of runs by id, after their writes committed.
pub fn publish_runs(core: &Arc<BridgeCore>, run_ids: &[String]) {
    let runs: Vec<wire::WorkflowRun> = {
        let db = core.db.lock().unwrap();
        run_ids.iter().filter_map(|id| get_run(&db, id).ok().flatten()).collect()
    };
    for run in runs {
        publish_run(core, run);
    }
}

/// One maintenance pass: reclaim, reconcile, claim what is due. Returns the
/// runs it claimed for the caller to dispatch.
pub fn tick(core: &Arc<BridgeCore>, now: DateTime<Utc>) -> Result<Vec<ClaimedRun>, BridgeError> {
    let mut changed_runs = Vec::new();
    let mut claimed = Vec::new();
    let changed_workflows = {
        let db = core.db.lock().unwrap();
        changed_runs.extend(reclaim_expired(&db, now)?);
        let changed_workflows = workflows::reconcile_projects(&db)?;
        // Parked runs go back to the runner, which re-checks the quota and
        // parks them again if their provider still has no room.
        claimed.extend(unpark_waiting(&db, now)?);
        for (workflow_id, occasion) in due_occasions(&db, now)? {
            match claim(&db, &workflow_id, &occasion, false, now)? {
                ClaimOutcome::Claimed(run) => {
                    changed_runs.push(run.run.id.clone());
                    claimed.push(run);
                }
                ClaimOutcome::Overlap { skipped: Some(skipped), .. } => changed_runs.push(skipped.id),
                ClaimOutcome::Overlap { skipped: None, .. } | ClaimOutcome::Existing(_) => {}
                ClaimOutcome::Refused(reason) => {
                    eprintln!("[bridge] workflow {workflow_id} slot not claimed: {reason}");
                }
            }
        }
        changed_workflows
    };
    for workflow_id in changed_workflows {
        core.events.publish(CoreEvent::WorkflowsChanged { workflow_id });
    }
    publish_runs(core, &changed_runs);
    Ok(claimed)
}

/// Hand a claimed run to the runner on its own thread.
pub fn dispatch(core: Arc<BridgeCore>, claimed: ClaimedRun) {
    std::thread::Builder::new()
        .name(format!("workflow-run-{}", claimed.run.id))
        .spawn(move || run_claimed(&core, claimed))
        .expect("spawn workflow run thread");
}

/// What a claimed run does.
fn run_claimed(core: &Arc<BridgeCore>, claimed: ClaimedRun) {
    crate::workflow_run::execute(core, claimed);
}

/// Start the loop on its own thread. Both hosts call this at boot.
pub fn start_workflow_maintenance(core: Arc<BridgeCore>) {
    std::thread::Builder::new()
        .name("workflow-maintenance".into())
        .spawn(move || loop {
            match tick(&core, Utc::now()) {
                Ok(claimed) => {
                    for run in claimed {
                        dispatch(core.clone(), run);
                    }
                }
                Err(error) => eprintln!("[bridge] workflow maintenance failed: {error}"),
            }
            std::thread::sleep(StdDuration::from_secs(TICK_SECONDS));
        })
        .expect("spawn workflow maintenance thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::test_support;

    fn at(text: &str) -> DateTime<Utc> {
        parse_instant(text).unwrap()
    }

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let connection = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, connection)
    }

    /// A workflow approved at a fixed instant, so slot counting is deterministic.
    fn approved(db: &Connection, dir: &std::path::Path, approved_at: &str, cron: &str) -> wire::Workflow {
        let mut definition = test_support::definition();
        definition.trigger = wire::WorkflowTrigger::Schedule { cron: cron.into(), timezone: "UTC".into() };
        let workflow = test_support::save_approved(db, dir, &definition);
        db.execute("UPDATE workflow_revisions SET approved_at=?2 WHERE id=?1", params![workflow.revision_id, approved_at]).unwrap();
        db.execute("UPDATE workflows SET updated_at=?2 WHERE id=?1", params![workflow.id, approved_at]).unwrap();
        workflows::require(db, &workflow.id).unwrap()
    }

    fn claimed(outcome: ClaimOutcome) -> ClaimedRun {
        match outcome {
            ClaimOutcome::Claimed(run) => run,
            other => panic!("expected a claim, got {other:?}"),
        }
    }

    #[test]
    fn the_same_occasion_fired_twice_yields_one_run() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        let now = at("2026-10-29T10:00:00Z");
        let occasion = Occasion::manual(&workflow.id, "r1");
        let first = claimed(claim(&db, &workflow.id, &occasion, false, now).unwrap());
        assert_eq!(first.run.status, wire::WorkflowRunStatus::Queued);
        match claim(&db, &workflow.id, &occasion, false, now).unwrap() {
            ClaimOutcome::Existing(run) => assert_eq!(run.id, first.run.id),
            other => panic!("{other:?}"),
        }
        let count: i64 = db.query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_claim_during_an_active_run_is_skipped_or_returned() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        let now = at("2026-10-29T09:00:30Z");
        let active = claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, now).unwrap());

        match claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r2"), false, now).unwrap() {
            ClaimOutcome::Overlap { active: returned, skipped: None } => assert_eq!(returned.id, active.run.id),
            other => panic!("{other:?}"),
        }
        let slot = Occasion::slot(&workflow.id, at("2026-10-29T09:00:00Z"), wire::WorkflowTriggerKind::Schedule, 0);
        match claim(&db, &workflow.id, &slot, false, now).unwrap() {
            ClaimOutcome::Overlap { skipped: Some(skipped), .. } => {
                assert_eq!(skipped.status, wire::WorkflowRunStatus::Skipped);
                assert_eq!(skipped.failure_code.as_deref(), Some("overlap"));
            }
            other => panic!("{other:?}"),
        }
        // The skipped slot is its occasion's run: it is not skipped twice.
        assert!(matches!(claim(&db, &workflow.id, &slot, false, now).unwrap(), ClaimOutcome::Existing(_)));
    }

    #[test]
    fn the_database_refuses_a_second_active_run() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, at("2026-10-29T10:00:00Z")).unwrap());
        let inserted = db.execute(
            "INSERT INTO workflow_runs(id,workflow_id,revision_id,trigger_kind,occasion,idempotency_key,status,created_at)
             VALUES('x',?1,?2,'manual','k','k','running','now')",
            params![workflow.id, workflow.revision_id],
        );
        assert!(inserted.is_err(), "the partial unique index forbids it");
    }

    #[test]
    fn an_expired_lease_is_reclaimed_and_the_stale_worker_cannot_commit() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        let start = at("2026-10-29T10:00:00Z");
        let run = claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, start).unwrap());

        assert!(reclaim_expired(&db, start + Duration::minutes(14)).unwrap().is_empty());
        let later = start + Duration::minutes(LEASE_MINUTES + 1);
        assert_eq!(reclaim_expired(&db, later).unwrap(), vec![run.run.id.clone()]);
        let reclaimed = require_run(&db, &run.run.id).unwrap();
        assert_eq!(reclaimed.status, wire::WorkflowRunStatus::Failed);
        assert_eq!(reclaimed.failure_code.as_deref(), Some("lease_expired"));

        assert!(!heartbeat(&db, &run.run.id, &run.lease_owner, later).unwrap());
        assert!(!finish(&db, &run.run.id, &run.lease_owner, wire::WorkflowRunStatus::Succeeded, &Outcome::default(), later).unwrap());
        assert_eq!(require_run(&db, &run.run.id).unwrap().status, wire::WorkflowRunStatus::Failed);
    }

    #[test]
    fn heartbeat_and_finish_require_the_live_lease() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        let start = at("2026-10-29T10:00:00Z");
        let run = claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, start).unwrap());
        assert!(!heartbeat(&db, &run.run.id, "someone-else", start).unwrap());
        assert!(heartbeat(&db, &run.run.id, &run.lease_owner, start + Duration::minutes(10)).unwrap());
        // The heartbeat moved the expiry, so a reclaim at the old expiry leaves it alone.
        assert!(reclaim_expired(&db, start + Duration::minutes(LEASE_MINUTES + 1)).unwrap().is_empty());
        assert!(transition(&db, &run.run.id, &run.lease_owner, wire::WorkflowRunStatus::Running, Some("s1"), start + Duration::minutes(11)).unwrap());
        let running = require_run(&db, &run.run.id).unwrap();
        assert_eq!(running.session_id.as_deref(), Some("s1"));
        assert!(running.started_at.is_some());
        let outcome = Outcome { summary: Some("done".into()), cost_microusd: Some(40_000), ..Outcome::default() };
        assert!(finish(&db, &run.run.id, &run.lease_owner, wire::WorkflowRunStatus::Succeeded, &outcome, start + Duration::minutes(12)).unwrap());
        let done = require_run(&db, &run.run.id).unwrap();
        assert_eq!(done.status, wire::WorkflowRunStatus::Succeeded);
        assert_eq!(done.cost_microusd, Some(40_000));
        assert!(finish(&db, &run.run.id, &run.lease_owner, wire::WorkflowRunStatus::Failed, &outcome, start + Duration::minutes(13)).is_ok_and(|written| !written));
    }

    #[test]
    fn cancel_settles_a_queued_run_and_clears_its_lease() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        let now = at("2026-10-29T10:00:00Z");
        let run = claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r1"), false, now).unwrap());
        let cancelled = cancel(&db, &run.run.id, now).unwrap();
        assert_eq!(cancelled.status, wire::WorkflowRunStatus::Cancelled);
        assert!(!heartbeat(&db, &run.run.id, &run.lease_owner, now).unwrap(), "the holder learns on its next heartbeat");
        assert!(cancel(&db, &run.run.id, now).unwrap_err().to_string().contains("already finished"));
        // With nothing active, the next Run now claims again.
        claimed(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "r2"), false, now).unwrap());
    }

    #[test]
    fn three_missed_slots_yield_one_catch_up_with_missed_slots_three() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-27T00:00:00Z", "0 9 * * *");
        // Asleep from the 27th to the 29th at noon: slots on the 27th, 28th and 29th.
        let now = at("2026-10-29T12:00:00Z");
        let due = due_occasions(&db, now).unwrap();
        assert_eq!(due.len(), 1);
        let (id, occasion) = &due[0];
        assert_eq!(id, &workflow.id);
        assert_eq!(occasion.kind, wire::WorkflowTriggerKind::CatchUp);
        assert_eq!(occasion.missed_slots, 3);
        assert_eq!(occasion.scheduled_for, Some(at("2026-10-29T09:00:00Z")));
        claimed(claim(&db, id, occasion, false, now).unwrap());
        assert!(due_occasions(&db, now).unwrap().is_empty(), "the slot is not offered again");

        // An on-time slot is a schedule run, not a catch-up.
        let next = at("2026-10-30T09:02:00Z");
        let run = latest_run(&db, &workflow.id).unwrap().unwrap();
        db.execute("UPDATE workflow_runs SET status='succeeded', lease_owner=NULL, lease_expires_at=NULL WHERE id=?1", params![run.id]).unwrap();
        let due = due_occasions(&db, next).unwrap();
        assert_eq!(due[0].1.kind, wire::WorkflowTriggerKind::Schedule);
        assert_eq!(due[0].1.missed_slots, 0);
    }

    #[test]
    fn slots_before_the_anchor_are_never_caught_up() {
        let (dir, db) = db();
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");
        // Turned off and back on at noon on the 29th: the 26th to 29th are not replayed.
        workflows::set_enabled(&db, &workflow.id, false).unwrap();
        workflows::set_enabled(&db, &workflow.id, true).unwrap();
        db.execute("UPDATE workflows SET updated_at='2026-10-29T12:00:00Z' WHERE id=?1", params![workflow.id]).unwrap();
        assert!(due_occasions(&db, at("2026-10-29T13:00:00Z")).unwrap().is_empty());

        // Unpausing is an anchor too.
        write_settings(&db, wire::WorkflowSettings { paused: true, quota_reserve_percent: 25 }, at("2026-10-29T13:00:00Z")).unwrap();
        assert!(due_occasions(&db, at("2026-10-30T10:00:00Z")).unwrap().is_empty(), "nothing is due while paused");
        write_settings(&db, wire::WorkflowSettings { paused: false, quota_reserve_percent: 25 }, at("2026-10-31T12:00:00Z")).unwrap();
        assert!(due_occasions(&db, at("2026-10-31T13:00:00Z")).unwrap().is_empty());
        assert_eq!(due_occasions(&db, at("2026-11-01T09:01:00Z")).unwrap().len(), 1);
    }

    #[test]
    fn paused_disabled_unapproved_and_removed_workflows_never_claim() {
        let (dir, db) = db();
        let now = at("2026-10-29T10:00:00Z");
        let workflow = approved(&db, dir.path(), "2026-10-26T00:00:00Z", "0 9 * * *");

        write_settings(&db, wire::WorkflowSettings { paused: true, quota_reserve_percent: 25 }, now).unwrap();
        assert!(matches!(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "a"), false, now).unwrap(), ClaimOutcome::Refused(reason) if reason.contains("paused")));
        write_settings(&db, wire::WorkflowSettings { paused: false, quota_reserve_percent: 25 }, now).unwrap();

        workflows::set_enabled(&db, &workflow.id, false).unwrap();
        assert!(matches!(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "b"), false, now).unwrap(), ClaimOutcome::Refused(reason) if reason.contains("turned off")));
        workflows::set_enabled(&db, &workflow.id, true).unwrap();

        let mut edited = test_support::definition();
        edited.grants[0].write = wire::WorkflowWriteAccess::Ask;
        workflows::save(&db, dir.path(), Some(&workflow.id), &edited, workflows::CreatedBy::User).unwrap();
        assert!(matches!(claim(&db, &workflow.id, &Occasion::manual(&workflow.id, "c"), false, now).unwrap(), ClaimOutcome::Refused(reason) if reason.contains("approval")));
        assert!(due_occasions(&db, at("2026-11-02T09:00:30Z")).unwrap().is_empty(), "an unapproved schedule is never due");

        // A project-bound workflow whose folder is gone.
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p1','repo',?1,'now')", params![repo.to_string_lossy()]).unwrap();
        let mut bound = test_support::definition();
        bound.project_id = Some("p1".into());
        let bound = test_support::save_approved(&db, dir.path(), &bound);
        std::fs::remove_dir_all(&repo).unwrap();
        assert!(matches!(claim(&db, &bound.id, &Occasion::manual(&bound.id, "d"), false, now).unwrap(), ClaimOutcome::Refused(reason) if reason.contains("project is gone")));
    }

    #[test]
    fn a_manual_dry_run_may_run_unapproved_and_a_schedule_may_not() {
        let (dir, db) = db();
        let now = at("2026-10-29T10:00:00Z");
        let saved = workflows::save(&db, dir.path(), None, &test_support::definition(), workflows::CreatedBy::User).unwrap().workflow;
        assert!(!saved.approved);
        let dry = claimed(claim(&db, &saved.id, &Occasion::manual(&saved.id, "dry"), true, now).unwrap());
        assert!(dry.run.dry_run);
        cancel(&db, &dry.run.id, now).unwrap();
        assert!(matches!(claim(&db, &saved.id, &Occasion::manual(&saved.id, "real"), false, now).unwrap(), ClaimOutcome::Refused(_)));
        let slot = Occasion::slot(&saved.id, at("2026-10-29T09:00:00Z"), wire::WorkflowTriggerKind::Schedule, 0);
        assert!(matches!(claim(&db, &saved.id, &slot, true, now).unwrap(), ClaimOutcome::Refused(_)));
    }

    #[test]
    fn run_now_and_schedules_insert_runs_only_through_claim() {
        // The only INSERT INTO workflow_runs outside tests is in this module's claim path.
        let sources = [
            ("workflow_trigger.rs", include_str!("workflow_trigger.rs")),
            ("workflows.rs", include_str!("workflows.rs")),
            ("api.rs", include_str!("api.rs")),
        ];
        for (name, source) in sources {
            let production = source.split("#[cfg(test)]").next().unwrap();
            let inserts = production.matches("INSERT INTO workflow_runs").count();
            let expected = if name == "workflow_trigger.rs" { 2 } else { 0 };
            assert_eq!(inserts, expected, "{name}");
        }
    }

    #[test]
    fn native_automations_are_never_dispatched() {
        // The engine reads only the workflows tables; the native catalog module
        // is never named by it.
        let production = include_str!("workflow_trigger.rs").split("#[cfg(test)]").next().unwrap();
        assert!(!production.contains("automations::"));
        assert!(!production.contains("scheduled_tasks"));
    }

    #[test]
    fn settings_round_trip_and_refuse_out_of_range_reserve() {
        let (_dir, db) = db();
        let now = at("2026-10-29T10:00:00Z");
        let fresh = read_settings(&db).unwrap();
        assert!(!fresh.configured);
        assert_eq!(fresh.settings.quota_reserve_percent, DEFAULT_QUOTA_RESERVE_PERCENT);
        let written = write_settings(&db, wire::WorkflowSettings { paused: true, quota_reserve_percent: 40 }, now).unwrap();
        assert!(written.configured);
        assert_eq!(written.settings, wire::WorkflowSettings { paused: true, quota_reserve_percent: 40 });
        for bad in [-1, 91] {
            assert!(write_settings(&db, wire::WorkflowSettings { paused: false, quota_reserve_percent: bad }, now).is_err());
        }
    }

    #[test]
    fn tick_claims_due_slots_and_unparks_waiting_runs() {
        let (_scratch, core) = test_support::core();
        let workflow = {
            let db = core.db.lock().unwrap();
            approved(&db, _scratch.path(), "2026-10-26T00:00:00Z", "0 9 * * *")
        };
        let mut events = core.events.subscribe();
        let claimed = tick(&core, at("2026-10-29T09:00:20Z")).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].run.workflow_id, workflow.id);
        let mut statuses = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::WorkflowRunChanged { run } = event {
                statuses.push(run.status);
            }
        }
        assert_eq!(statuses, vec![wire::WorkflowRunStatus::Queued]);

        // Parked for quota: no lease, still active, and the next tick hands it back.
        let db = core.db.lock().unwrap();
        assert!(park(&db, &claimed[0].run.id, &claimed[0].lease_owner).unwrap());
        let parked = require_run(&db, &claimed[0].run.id).unwrap();
        assert_eq!(parked.status, wire::WorkflowRunStatus::WaitingQuota);
        assert!(reclaim_expired(&db, at("2026-10-30T09:00:00Z")).unwrap().is_empty(), "a parked run holds no lease to expire");
        let back = unpark_waiting(&db, at("2026-10-29T09:01:00Z")).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].run.status, wire::WorkflowRunStatus::Queued);
        assert!(heartbeat(&db, &back[0].run.id, &back[0].lease_owner, at("2026-10-29T09:02:00Z")).unwrap());
    }
}
