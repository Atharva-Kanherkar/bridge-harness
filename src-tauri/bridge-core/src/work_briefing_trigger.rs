//! One claim path for every briefing trigger.
//!
//! Manual, focus, and cadence all arrive here, and no provider starts any other
//! way. That single entrance is what makes the guarantees checkable: at most one
//! active run, a lease that outlives a crash, an idempotency key per occasion,
//! and a cooldown that no caller can forget to apply.
//!
//! The lease shape is `learning_job.rs`'s, deliberately: a 15-minute lease,
//! heartbeated by the worker that holds it, reclaimed by compare-and-swap once
//! it expires. Zero rows changed on the CAS means another trigger won, and the
//! loser observes rather than racing.

use bridge_protocol::messages as wire;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::work;
use crate::work_brief_store::{begin_run, RunStart};
use crate::work_briefing_config::{resolve_briefing, BriefingSelection};
use crate::BridgeError;

/// How long a claim holds without a heartbeat. The run's wall deadline is
/// enforced inside the run loop; this is the crash boundary, not the budget.
pub const LEASE_MINUTES: i64 = 15;

/// A run this trigger now owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRun {
    pub run_id: String,
    /// The token every later write must present. A worker that lost its lease
    /// keeps the token but the row no longer matches it — which is the whole
    /// mechanism behind "a stale worker cannot commit".
    pub lease_owner: String,
    pub trigger: wire::WorkBriefTrigger,
    pub selection: BriefingSelection,
    pub settings: wire::WorkSettings,
}

/// What a trigger got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed(ClaimedRun),
    /// Another trigger's run is active. Not an error: the racing trigger's job
    /// is done the moment somebody is running.
    Observed { run_id: String },
    /// Nothing ran and nothing will, with a stable code saying why.
    Refused { code: String, detail: String },
}

fn refused(code: &str, detail: impl Into<String>) -> ClaimOutcome {
    ClaimOutcome::Refused { code: code.into(), detail: detail.into() }
}

/// Claim the right to run one briefing.
///
/// `reported_versions` is what each registered harness reports right now — the
/// caller passes the adapter registry's view so this module stays store-only.
pub fn claim(
    db: &Connection,
    trigger: wire::WorkBriefTrigger,
    reported_versions: &dyn Fn(&str) -> Option<String>,
    now: DateTime<Utc>,
) -> Result<ClaimOutcome, BridgeError> {
    let snapshot = work::read_settings(db)?;
    if !snapshot.configured {
        return Ok(refused("not_configured", "Work has never been configured"));
    }
    let settings = snapshot.settings;
    let selection = match resolve_briefing(&settings, reported_versions) {
        Ok(selection) => selection,
        Err(unavailable) => return Ok(refused(unavailable.code(), unavailable.reason())),
    };

    // One active run. An unexpired lease is observed; an expired one is settled
    // by CAS so exactly one racing trigger performs the settlement.
    let active: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT id,lease_expires_at FROM work_brief_runs
              WHERE status='running' ORDER BY started_at DESC, rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((run_id, lease_expires_at)) = active {
        let expired = lease_expires_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|expiry| expiry <= now);
        if !expired {
            return Ok(ClaimOutcome::Observed { run_id });
        }
        // `IS`, not `=`: a legacy row with no lease must still be settleable,
        // and `= NULL` matches nothing.
        let settled = db.execute(
            "UPDATE work_brief_runs SET
                 status='failed',failure_code='lease_expired',
                 failure_detail='the lease expired without a completion; a later trigger settled the run',
                 completed_at=?2,lease_owner=NULL,lease_expires_at=NULL
               WHERE id=?1 AND status='running' AND lease_expires_at IS ?3",
            params![run_id, now.to_rfc3339(), lease_expires_at],
        )?;
        if settled == 0 {
            // Another trigger settled or re-leased it between our read and our
            // write. Either way somebody else is ahead; observe them.
            return Ok(ClaimOutcome::Observed { run_id });
        }
    }

    // Cooldown guards the two unattended triggers. A human pressing Refresh is
    // not a loop, so manual is exempt — the one-active-run rule above still
    // holds it to a single run at a time.
    if trigger != wire::WorkBriefTrigger::Manual && settings.cooldown_minutes > 0 {
        let last_started: Option<String> = db
            .query_row(
                "SELECT started_at FROM work_brief_runs ORDER BY started_at DESC, rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(started) = last_started.as_deref().and_then(|value| DateTime::parse_from_rfc3339(value).ok()) {
            let elapsed = now.signed_duration_since(started);
            if elapsed < Duration::minutes(settings.cooldown_minutes) {
                return Ok(refused(
                    "cooldown",
                    format!(
                        "the last run started {} minute(s) ago; the cooldown is {} minute(s)",
                        elapsed.num_minutes().max(0),
                        settings.cooldown_minutes
                    ),
                ));
            }
        }
    }

    // The occasion key. Deterministic per cause, so a race on the same cause
    // collapses on the schema's partial unique index rather than double-running.
    let idempotency_key = match trigger {
        wire::WorkBriefTrigger::Manual => None,
        wire::WorkBriefTrigger::Focus => {
            let bucket_minutes = settings.cooldown_minutes.max(1);
            Some(format!("focus:{}", now.timestamp() / (bucket_minutes * 60)))
        }
        wire::WorkBriefTrigger::Schedule => {
            let Some(interval) = settings.refresh_interval_minutes else {
                return Ok(refused("cadence_disabled", "no refresh interval is configured"));
            };
            Some(format!("schedule:{}", now.timestamp() / (interval.max(1) * 60)))
        }
    };

    let run_id = format!("run-{}", Uuid::new_v4());
    let lease_owner = Uuid::new_v4().to_string();
    let inserted = begin_run(
        db,
        &RunStart {
            run_id: run_id.clone(),
            trigger,
            profile_reference: Some(selection.reference()),
            session_id: None,
            limits: settings.limits,
            idempotency_key: idempotency_key.clone(),
            started_at: now.to_rfc3339(),
            lease_owner: Some(lease_owner.clone()),
            lease_expires_at: Some((now + Duration::minutes(LEASE_MINUTES)).to_rfc3339()),
        },
    );
    if inserted.is_err() {
        // The one insert failure this path expects: the occasion was already
        // claimed. Anything else propagates as the database error it is.
        if let Some(existing) = idempotency_key.as_deref().and_then(|key| {
            db.query_row(
                "SELECT id FROM work_brief_runs WHERE idempotency_key=?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
        }) {
            return Ok(ClaimOutcome::Observed { run_id: existing });
        }
        inserted?;
    }
    Ok(ClaimOutcome::Claimed(ClaimedRun {
        run_id,
        lease_owner,
        trigger,
        selection,
        settings,
    }))
}

/// Extend a held lease. Returns whether the caller still holds it.
pub fn heartbeat(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let extended = db.execute(
        "UPDATE work_brief_runs SET lease_expires_at=?3
           WHERE id=?1 AND lease_owner=?2 AND status='running'",
        params![run_id, lease_owner, (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339()],
    )?;
    Ok(extended == 1)
}

/// The pre-commit assertion: still the owner, still running, lease not yet
/// expired. The expiry condition is what makes this exclusive with a reclaim —
/// the reclaimer requires `expiry <= now`, this requires `expiry > now`, and
/// SQLite serialises the two writes on the same row.
pub fn assert_lease_for_commit(
    db: &Connection,
    run_id: &str,
    lease_owner: &str,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let held = db.execute(
        "UPDATE work_brief_runs SET lease_expires_at=?3
           WHERE id=?1 AND lease_owner=?2 AND status='running'
             AND julianday(lease_expires_at) > julianday(?4)",
        params![
            run_id,
            lease_owner,
            (now + Duration::minutes(LEASE_MINUTES)).to_rfc3339(),
            now.to_rfc3339()
        ],
    )?;
    Ok(held == 1)
}

/// Drop the lease once the run row is terminal.
pub fn release(db: &Connection, run_id: &str, lease_owner: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE work_brief_runs SET lease_owner=NULL,lease_expires_at=NULL
           WHERE id=?1 AND lease_owner=?2",
        params![run_id, lease_owner],
    )?;
    Ok(())
}

/// Ask the active run to stop. Returns its id, or `None` when nothing is running.
pub fn request_cancel(db: &Connection) -> Result<Option<String>, BridgeError> {
    let active: Option<String> = db
        .query_row(
            "SELECT id FROM work_brief_runs WHERE status='running'
              ORDER BY started_at DESC, rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(run_id) = active.as_deref() {
        db.execute(
            "UPDATE work_brief_runs SET cancellation_requested=1 WHERE id=?1",
            params![run_id],
        )?;
    }
    Ok(active)
}

/// Has somebody asked this run to stop? Polled from the run loop.
pub fn cancellation_requested(db: &Connection, run_id: &str) -> Result<bool, BridgeError> {
    let requested: Option<i64> = db
        .query_row(
            "SELECT cancellation_requested FROM work_brief_runs WHERE id=?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(requested.unwrap_or(0) != 0)
}

/// Record which hidden session serves this run, once the live path creates it.
pub fn record_session(db: &Connection, run_id: &str, session_id: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE work_brief_runs SET session_id=?2 WHERE id=?1",
        params![run_id, session_id],
    )?;
    Ok(())
}

/// Is a cadence run due? The maintenance thread's cheap pre-check; `claim`
/// re-verifies everything, so a stale answer here costs one refused claim.
pub fn schedule_due(db: &Connection, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    let snapshot = work::read_settings(db)?;
    if !snapshot.configured || snapshot.settings.briefing.is_none() {
        return Ok(false);
    }
    let Some(interval) = snapshot.settings.refresh_interval_minutes else {
        return Ok(false);
    };
    let last_started: Option<String> = db
        .query_row(
            "SELECT started_at FROM work_brief_runs ORDER BY started_at DESC, rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match last_started.as_deref().and_then(|value| DateTime::parse_from_rfc3339(value).ok()) {
        Some(started) => now.signed_duration_since(started) >= Duration::minutes(interval),
        None => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use crate::work_brief_store::{finish_run, read_run, RunOutcome};

    fn db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn versions(harness: &str) -> Option<String> {
        (harness == "claude").then(|| "0.3.209".to_owned())
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-19T12:00:00+00:00").unwrap().with_timezone(&Utc)
    }

    fn configure(db: &Connection, cooldown_minutes: i64, interval: Option<i64>) {
        let mut settings = work::default_settings();
        settings.briefing = Some(wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse("claude").unwrap(),
            model: "haiku".into(),
            effort: None,
        });
        settings.cooldown_minutes = cooldown_minutes;
        settings.refresh_interval_minutes = interval;
        work::write_settings(db, &settings).unwrap();
    }

    fn finish(db: &Connection, run_id: &str, at: &str) {
        finish_run(
            db,
            run_id,
            &RunOutcome {
                status: wire::WorkBriefRunStatus::Succeeded,
                output_digest: None,
                failure_code: None,
                failure_detail: None,
                usage: None,
                tool_calls: 0,
                turns: 1,
                completed_at: at.into(),
            },
        )
        .unwrap();
        db.execute(
            "UPDATE work_brief_runs SET lease_owner=NULL,lease_expires_at=NULL WHERE id=?1",
            params![run_id],
        )
        .unwrap();
    }

    fn claimed(outcome: ClaimOutcome) -> ClaimedRun {
        match outcome {
            ClaimOutcome::Claimed(run) => run,
            other => panic!("expected a claim, got {other:?}"),
        }
    }

    #[test]
    fn an_unconfigured_install_is_refused_with_the_stable_code() {
        let db = db();
        let outcome = claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap();
        assert_eq!(
            outcome,
            ClaimOutcome::Refused {
                code: "not_configured".into(),
                detail: "Work has never been configured".into()
            }
        );
    }

    #[test]
    fn briefing_switched_off_is_refused_not_run() {
        let db = db();
        work::write_settings(&db, &work::default_settings()).unwrap();
        let outcome = claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap();
        let ClaimOutcome::Refused { code, .. } = outcome else {
            panic!("briefing off must refuse");
        };
        assert_eq!(code, "not_configured");
    }

    #[test]
    fn racing_triggers_start_at_most_one_run_and_the_losers_observe_it() {
        let db = db();
        configure(&db, 0, Some(60));
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        for trigger in [
            wire::WorkBriefTrigger::Manual,
            wire::WorkBriefTrigger::Focus,
            wire::WorkBriefTrigger::Schedule,
        ] {
            let outcome = claim(&db, trigger, &versions, now()).unwrap();
            assert_eq!(
                outcome,
                ClaimOutcome::Observed { run_id: run.run_id.clone() },
                "{trigger:?} must observe the active run"
            );
        }
    }

    #[test]
    fn an_expired_lease_is_settled_once_and_the_run_reads_as_failed() {
        let db = db();
        configure(&db, 0, None);
        let stale = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        db.execute(
            "UPDATE work_brief_runs SET lease_expires_at=?2 WHERE id=?1",
            params![stale.run_id, "2026-08-19T11:00:00+00:00"],
        )
        .unwrap();

        let next = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        assert_ne!(next.run_id, stale.run_id);
        let settled = read_run(&db, &stale.run_id).unwrap().unwrap();
        assert_eq!(settled.status, wire::WorkBriefRunStatus::Failed);
        assert_eq!(settled.failure_code.as_deref(), Some("lease_expired"));
    }

    #[test]
    fn a_stale_worker_cannot_commit_after_a_reclaim() {
        let db = db();
        configure(&db, 0, None);
        let stale = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        db.execute(
            "UPDATE work_brief_runs SET lease_expires_at=?2 WHERE id=?1",
            params![stale.run_id, "2026-08-19T11:00:00+00:00"],
        )
        .unwrap();
        claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());

        assert!(
            !assert_lease_for_commit(&db, &stale.run_id, &stale.lease_owner, now()).unwrap(),
            "the settled run's owner no longer holds anything"
        );
    }

    #[test]
    fn the_precommit_assertion_holds_for_a_live_lease_and_extends_it() {
        let db = db();
        configure(&db, 0, None);
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        assert!(assert_lease_for_commit(&db, &run.run_id, &run.lease_owner, now()).unwrap());
        assert!(
            !assert_lease_for_commit(&db, &run.run_id, "someone-else", now()).unwrap(),
            "only the owner may commit"
        );
    }

    #[test]
    fn a_heartbeat_extends_only_the_owners_running_lease() {
        let db = db();
        configure(&db, 0, None);
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        assert!(heartbeat(&db, &run.run_id, &run.lease_owner, now()).unwrap());
        assert!(!heartbeat(&db, &run.run_id, "someone-else", now()).unwrap());
        finish(&db, &run.run_id, "2026-08-19T12:01:00+00:00");
        assert!(
            !heartbeat(&db, &run.run_id, &run.lease_owner, now()).unwrap(),
            "a terminal run has no lease to extend"
        );
    }

    #[test]
    fn the_cooldown_holds_focus_and_schedule_but_never_manual() {
        let db = db();
        configure(&db, 15, Some(60));
        let first = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        finish(&db, &first.run_id, "2026-08-19T12:01:00+00:00");

        let five_later = now() + Duration::minutes(5);
        for trigger in [wire::WorkBriefTrigger::Focus, wire::WorkBriefTrigger::Schedule] {
            let ClaimOutcome::Refused { code, .. } =
                claim(&db, trigger, &versions, five_later).unwrap()
            else {
                panic!("{trigger:?} inside the cooldown must be refused");
            };
            assert_eq!(code, "cooldown");
        }
        claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, five_later).unwrap());
    }

    #[test]
    fn a_schedule_occasion_is_claimed_at_most_once() {
        let db = db();
        configure(&db, 0, Some(60));
        let first = claimed(claim(&db, wire::WorkBriefTrigger::Schedule, &versions, now()).unwrap());
        finish(&db, &first.run_id, "2026-08-19T12:00:30+00:00");

        // Same cadence bucket, run already terminal: the unique key is what
        // refuses the double, and the caller observes the run that claimed it.
        let again = claim(&db, wire::WorkBriefTrigger::Schedule, &versions, now() + Duration::seconds(30)).unwrap();
        assert_eq!(again, ClaimOutcome::Observed { run_id: first.run_id });
    }

    #[test]
    fn a_schedule_trigger_with_no_cadence_is_refused() {
        let db = db();
        configure(&db, 0, None);
        let ClaimOutcome::Refused { code, .. } =
            claim(&db, wire::WorkBriefTrigger::Schedule, &versions, now()).unwrap()
        else {
            panic!("no cadence, no schedule run");
        };
        assert_eq!(code, "cadence_disabled");
    }

    #[test]
    fn cancellation_is_requested_on_the_active_run_and_polled_by_id() {
        let db = db();
        configure(&db, 0, None);
        assert_eq!(request_cancel(&db).unwrap(), None, "nothing running, nothing to cancel");
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        assert!(!cancellation_requested(&db, &run.run_id).unwrap());
        assert_eq!(request_cancel(&db).unwrap(), Some(run.run_id.clone()));
        assert!(cancellation_requested(&db, &run.run_id).unwrap());
    }

    #[test]
    fn a_cadence_is_due_when_the_interval_has_passed_and_not_before() {
        let db = db();
        assert!(!schedule_due(&db, now()).unwrap(), "unconfigured is never due");
        configure(&db, 0, Some(60));
        assert!(schedule_due(&db, now()).unwrap(), "never run: due now");
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Schedule, &versions, now()).unwrap());
        finish(&db, &run.run_id, "2026-08-19T12:02:00+00:00");
        assert!(!schedule_due(&db, now() + Duration::minutes(30)).unwrap());
        assert!(schedule_due(&db, now() + Duration::minutes(61)).unwrap());
    }

    #[test]
    fn the_claim_records_the_selection_and_the_lease_on_the_run_row() {
        let db = db();
        configure(&db, 0, None);
        let run = claimed(claim(&db, wire::WorkBriefTrigger::Manual, &versions, now()).unwrap());
        assert_eq!(run.selection.harness, "claude");
        assert_eq!(run.selection.model, "haiku");
        let stored = read_run(&db, &run.run_id).unwrap().unwrap();
        assert_eq!(stored.status, wire::WorkBriefRunStatus::Running);
        assert_eq!(stored.profile_reference.as_deref(), Some("claude/haiku"));
        let (owner, expiry): (Option<String>, Option<String>) = db
            .query_row(
                "SELECT lease_owner,lease_expires_at FROM work_brief_runs WHERE id=?1",
                params![run.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(owner.as_deref(), Some(run.lease_owner.as_str()));
        assert!(expiry.is_some());
    }
}
