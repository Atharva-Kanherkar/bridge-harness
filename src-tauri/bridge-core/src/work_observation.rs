//! The half of Work's facts that cannot be read from SQLite.
//!
//! Base-branch divergence is measured by running git, which the Work board's
//! read path must never do. So it is measured here — on a maintenance thread,
//! and write-through whenever the user asks for a divergence reading anyway —
//! and stamped into `work_fact_cache` with the instant it was observed.
//! [`crate::work`] reads that cache and reports the age.
//!
//! This module is deliberately separate from [`crate::work`]: keeping the
//! subprocess on this side of the file boundary is what makes "the board is
//! store-only" something a test can check rather than something a comment
//! claims.

use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{git, work, BridgeCore, BridgeError};
use std::sync::Arc;

/// How often a workspace is re-measured. Well inside
/// [`work::WORK_FACT_STALE_AFTER_SECONDS`], so a board that is being looked at
/// normally shows `live` numbers and only goes stale when the observer itself
/// has stopped running.
pub const REFRESH_INTERVAL_SECONDS: i64 = 300;

/// How often the maintenance pass wakes up. The pass is cheap when nothing is
/// due — one indexed read — and the per-workspace interval above is what keeps
/// git from being run on a timer.
const MAINTENANCE_TICK: Duration = Duration::from_secs(60);

/// Store one observation. A failure records *that* it failed and keeps no
/// numbers: leaving the previous payload in place would let a stale reading
/// masquerade as a fresh one, which is the whole thing this cache exists to
/// stop.
pub fn record_base_divergence(
    db: &Connection,
    workspace_id: &str,
    observation: Result<&git::BaseBranchDivergence, &str>,
    observed_at: DateTime<Utc>,
) -> Result<(), BridgeError> {
    // Git runs outside the database lock, so a slow observation that started
    // first can finish after a reading the user asked for. Whoever measured most
    // recently wins: otherwise an in-flight observer silently replaces a fresher
    // write-through — including one measured against a freshly fetched ref — and
    // stamps its own older numbers with a newer time, so they read as `live`.
    //
    // Compared in Rust for the same reason `due_workspaces` compares ages there:
    // it must not depend on SQLite agreeing with chrono about timestamp formats.
    let stored: Option<String> = db
        .query_row(
            "SELECT observed_at FROM work_fact_cache WHERE kind=?1 AND cache_key=?2",
            params![work::FACT_CACHE_BASE_DIVERGENCE, workspace_id],
            |row| row.get(0),
        )
        .optional()?;
    if stored
        .as_deref()
        .and_then(|seen| DateTime::parse_from_rfc3339(seen).ok())
        .is_some_and(|seen| seen.with_timezone(&Utc) > observed_at)
    {
        return Ok(());
    }

    let (status, payload, detail) = match observation {
        // A reading that could compare nothing is not a successful observation
        // of "no drift". `should_warn` is false without a base ref, so storing
        // this as `ok` would have the projection drop it silently and a
        // workspace Bridge can no longer measure would look fine. No comparison
        // means nothing is known, which is what `failed` says.
        Ok(divergence) if divergence.unavailable_reason.is_some() => {
            ("failed", None, divergence.unavailable_reason.clone())
        }
        Ok(divergence) => (
            "ok",
            Some(serde_json::to_string(divergence).map_err(|error| {
                BridgeError::Invalid(format!("divergence is not serializable: {error}"))
            })?),
            None,
        ),
        Err(reason) => ("failed", None, Some(reason.to_owned())),
    };
    db.execute(
        "INSERT INTO work_fact_cache(kind,cache_key,status,payload,detail,observed_at)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(kind,cache_key) DO UPDATE SET
             status=excluded.status,payload=excluded.payload,
             detail=excluded.detail,observed_at=excluded.observed_at",
        params![
            work::FACT_CACHE_BASE_DIVERGENCE,
            workspace_id,
            status,
            payload,
            detail,
            observed_at.to_rfc3339()
        ],
    )?;
    Ok(())
}

/// Workspaces whose divergence is due for a look: they have a session (so the
/// fact would have somewhere to send a human), and their last observation is
/// missing or older than [`REFRESH_INTERVAL_SECONDS`].
fn due_workspaces(
    db: &Connection,
    now: DateTime<Utc>,
) -> Result<Vec<(String, String)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT w.id,w.path,
                (SELECT c.observed_at FROM work_fact_cache c
                  WHERE c.kind=?1 AND c.cache_key=w.id)
           FROM workspaces w
          WHERE EXISTS(SELECT 1 FROM sessions s WHERE s.workspace_id=w.id)
          ORDER BY w.id",
    )?;
    let rows = statement.query_map([work::FACT_CACHE_BASE_DIVERGENCE], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut due = Vec::new();
    for row in rows {
        let (workspace_id, path, observed_at) = row?;
        // The age comparison happens here rather than in SQL so it does not
        // depend on SQLite agreeing with chrono about timestamp formats.
        let fresh = observed_at
            .as_deref()
            .and_then(|seen| DateTime::parse_from_rfc3339(seen).ok())
            .is_some_and(|seen| {
                now.signed_duration_since(seen.with_timezone(&Utc))
                    .num_seconds()
                    < REFRESH_INTERVAL_SECONDS
            });
        if !fresh {
            due.push((workspace_id, path));
        }
    }
    Ok(due)
}

/// Re-measure every workspace that is due, and cache the result.
///
/// Resolves what to look at under the lock and runs git outside it, the same way
/// `api::workspace_base_divergence` does: a measurement can be slow, and holding
/// the database while it runs would stall every other reader.
pub fn refresh_base_divergence(core: &Arc<BridgeCore>) {
    let now = Utc::now();
    let due = {
        let db = core.db.lock().unwrap();
        match due_workspaces(&db, now) {
            Ok(due) => due,
            Err(_) => return,
        }
    };
    for (workspace_id, _) in due {
        let path: Option<String> = {
            let operation = core.workspace_operation(&workspace_id);
            let _operation = crate::runtime::lock_operation(&operation);
            core.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT path FROM workspaces WHERE id=?1",
                    params![workspace_id],
                    |row| row.get(0),
                )
                .optional()
                .ok()
                .flatten()
        };
        let Some(path) = path else {
            continue;
        };
        let path = std::path::PathBuf::from(path);
        // No fetch: the observer must not put the network on a background timer.
        // Git runs without the workspace operation lock so a slow status cannot
        // stall checkout or chat start.
        let observation = if path.is_dir() {
            Ok(git::base_branch_divergence(&path, false))
        } else {
            Err("the workspace directory is gone")
        };
        let observed_at = Utc::now();
        let db = core.db.lock().unwrap();
        let _ = record_base_divergence(
            &db,
            &workspace_id,
            observation.as_ref().map_err(|reason| *reason),
            observed_at,
        );
    }
}

/// Keep the Work fact cache observed. Its own thread, because the one-second
/// worker-pool loop must not be made to wait on a git subprocess.
pub fn start_work_fact_maintenance(core: Arc<BridgeCore>) {
    thread::spawn(move || loop {
        thread::sleep(MAINTENANCE_TICK);
        refresh_base_divergence(&core);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use bridge_protocol::messages as wire;

    fn memory_db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn seed(db: &Connection, workspace_path: &str) {
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('s',NULL,'codex','Codex','working','reported');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Kyoto','Task','bridge/task',?1,'idle','now')",
            params![workspace_path],
        )
        .unwrap();
        db.execute("UPDATE sessions SET workspace_id='w' WHERE id='s'", [])
            .unwrap();
    }

    fn divergence(behind: i64) -> git::BaseBranchDivergence {
        git::BaseBranchDivergence {
            base_ref: Some("origin/main".into()),
            base_commit: Some("abc123".into()),
            head: Some("def456".into()),
            branch: Some("bridge/task".into()),
            ahead: 1,
            behind,
            ref_age_seconds: Some(3_600),
            fetch_attempted: false,
            fetched: false,
            dirty: false,
            unavailable_reason: None,
        }
    }

    fn at(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn cached(db: &Connection) -> (String, Option<String>, Option<String>, String) {
        db.query_row(
            "SELECT status,payload,detail,observed_at FROM work_fact_cache
              WHERE kind=?1 AND cache_key='w'",
            params![work::FACT_CACHE_BASE_DIVERGENCE],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
    }

    #[test]
    fn a_successful_observation_is_cached_with_its_timestamp() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        let (status, payload, detail, observed_at) = cached(&db);
        assert_eq!(status, "ok");
        assert_eq!(detail, None);
        assert_eq!(observed_at, "2026-08-19T09:00:00+00:00");
        let stored: git::BaseBranchDivergence =
            serde_json::from_str(&payload.expect("payload")).unwrap();
        assert_eq!(stored.behind, 31);

        // Read back through the board: fresh, and offering the fast-forward.
        let facts = work::facts(&db, at("2026-08-19T09:05:00+00:00")).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].kind, wire::WorkFactKind::WorkspaceBehindBase);
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Live);
        assert_eq!(facts[0].observed_at, "2026-08-19T09:00:00+00:00");
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::RefreshWorkspaceBase {
                session_id: "s".into(),
                workspace_id: "w".into()
            }
        );
    }

    #[test]
    fn an_observation_past_the_staleness_window_reads_as_stale_and_offers_re_measurement() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        let facts = work::facts(&db, at("2026-08-19T09:20:00+00:00")).unwrap();
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Stale);
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::RefreshBaseObservation {
                session_id: "s".into(),
                workspace_id: "w".into()
            },
            "a number nobody has checked recently is not a number to act on"
        );
    }

    #[test]
    fn a_failed_observation_is_recorded_as_failed_and_keeps_no_numbers() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        record_base_divergence(
            &db,
            "w",
            Err("the workspace directory is gone"),
            at("2026-08-19T09:05:00+00:00"),
        )
        .unwrap();
        let (status, payload, detail, observed_at) = cached(&db);
        assert_eq!(status, "failed");
        assert_eq!(payload, None, "the previous reading must not survive as if it were current");
        assert_eq!(detail.as_deref(), Some("the workspace directory is gone"));
        assert_eq!(observed_at, "2026-08-19T09:05:00+00:00");

        let facts = work::facts(&db, at("2026-08-19T09:06:00+00:00")).unwrap();
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Unknown);
        assert!(facts[0].title.contains("could not be measured"));
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::RefreshBaseObservation {
                session_id: "s".into(),
                workspace_id: "w".into()
            }
        );
    }

    #[test]
    fn a_missing_observation_projects_nothing_at_all() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        assert!(
            work::facts(&db, at("2026-08-19T09:00:00+00:00")).unwrap().is_empty(),
            "never measured is not the same as up to date"
        );
    }

    #[test]
    fn a_workspace_close_to_its_base_is_nobody_s_problem() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(2)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        assert!(work::facts(&db, at("2026-08-19T09:01:00+00:00")).unwrap().is_empty());
    }

    #[test]
    fn an_unreadable_payload_reads_as_unknown_rather_than_as_zero_drift() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        db.execute(
            "INSERT INTO work_fact_cache(kind,cache_key,status,payload,observed_at)
             VALUES(?1,'w','ok','{\"behind\":\"lots\"}','2026-08-19T09:00:00+00:00')",
            params![work::FACT_CACHE_BASE_DIVERGENCE],
        )
        .unwrap();
        let facts = work::facts(&db, at("2026-08-19T09:01:00+00:00")).unwrap();
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Unknown);
    }

    #[test]
    fn a_slower_observation_never_clobbers_a_newer_reading() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        // What the user asked for, measured at 09:05.
        record_base_divergence(&db, "w", Ok(&divergence(44)), at("2026-08-19T09:05:00+00:00"))
            .unwrap();
        // An observer that started earlier finishing later. Its git ran outside
        // the lock, so it arrives second with older numbers.
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:04:00+00:00"))
            .unwrap();

        let (status, payload, _, observed_at) = cached(&db);
        assert_eq!(status, "ok");
        assert_eq!(
            observed_at, "2026-08-19T09:05:00+00:00",
            "the newer reading keeps its own timestamp, so it cannot read as fresher than it is"
        );
        let stored: git::BaseBranchDivergence =
            serde_json::from_str(&payload.expect("the newer payload survives")).unwrap();
        assert_eq!(stored.behind, 44, "and its numbers");

        // A late failure must not discard a newer good reading either.
        record_base_divergence(&db, "w", Err("git went away"), at("2026-08-19T09:04:30+00:00"))
            .unwrap();
        let (status, _, _, observed_at) = cached(&db);
        assert_eq!(status, "ok");
        assert_eq!(observed_at, "2026-08-19T09:05:00+00:00");
    }

    #[test]
    fn a_reading_that_compared_nothing_is_a_failure_not_an_absence_of_drift() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        let mut unavailable = divergence(0);
        unavailable.base_ref = None;
        unavailable.base_commit = None;
        unavailable.unavailable_reason =
            Some("no upstream or default branch ref is available".to_owned());

        record_base_divergence(&db, "w", Ok(&unavailable), at("2026-08-19T09:00:00+00:00"))
            .unwrap();

        let (status, payload, detail, _) = cached(&db);
        assert_eq!(
            status, "failed",
            "git ran but compared nothing, which is not a successful reading of zero drift"
        );
        assert!(payload.is_none(), "there are no numbers to keep");
        assert_eq!(
            detail.as_deref(),
            Some("no upstream or default branch ref is available"),
            "and the reason reaches the reader"
        );

        // The board must say it does not know, rather than silently omitting a
        // workspace it can no longer measure.
        let facts = work::facts(&db, at("2026-08-19T09:01:00+00:00")).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Unknown);
        assert!(matches!(
            facts[0].action,
            wire::WorkFactAction::RefreshBaseObservation { .. }
        ));
    }

    #[test]
    fn observing_twice_updates_in_place() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        record_base_divergence(&db, "w", Ok(&divergence(44)), at("2026-08-19T09:05:00+00:00"))
            .unwrap();
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM work_fact_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "one observation per workspace, or freshness is ambiguous");
        let (_, payload, _, observed_at) = cached(&db);
        let stored: git::BaseBranchDivergence =
            serde_json::from_str(&payload.unwrap()).unwrap();
        assert_eq!(stored.behind, 44);
        assert_eq!(observed_at, "2026-08-19T09:05:00+00:00");
    }

    #[test]
    fn a_divergence_reading_the_user_asked_for_is_written_through() {
        // The board reads observations, so a measurement someone already paid for
        // should land in the cache rather than being computed and dropped.
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let repository = data_dir.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "test@bridge.invalid"],
            vec!["config", "user.name", "Bridge Test"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["commit", "--allow-empty", "-q", "-m", "root"],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&repository)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }

        {
            let db = store::open(&data_dir.join("bridge.db")).unwrap();
            db.execute_batch(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
                 INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                     VALUES('s',NULL,'codex','Codex','idle','reported');",
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','main',?1,'idle','now')",
                params![repository.to_string_lossy()],
            )
            .unwrap();
            db.execute("UPDATE sessions SET workspace_id='w' WHERE id='s'", []).unwrap();
        }

        let core = Arc::new(
            crate::BridgeCore::boot(crate::BootConfig {
                data_dir: data_dir.to_path_buf(),
                browser_extension_path: data_dir.join("no-extension"),
                events: None,
            })
            .unwrap(),
        );
        let divergence = crate::api::workspace_base_divergence(&core, "s", false).unwrap();

        let db = core.db.lock().unwrap();
        let (status, payload, _, observed_at) = cached(&db);
        assert_eq!(status, "ok");
        assert!(!observed_at.is_empty());
        let stored: git::BaseBranchDivergence =
            serde_json::from_str(&payload.expect("the reading was cached")).unwrap();
        assert_eq!(stored, divergence, "the cache holds exactly what the caller was told");
    }

    /// Issue #306: the board showed a fresh measurement of the workspace root
    /// while the card's action failed with "not a git repository", because the
    /// action ran git in the session's cwd. A workspace session must be
    /// measured at its workspace root no matter where its cwd points.
    #[test]
    fn a_measurement_for_a_workspace_session_measures_the_workspace_root() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let repository = data_dir.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "test@bridge.invalid"],
            vec!["config", "user.name", "Bridge Test"],
            vec!["commit", "--allow-empty", "-q", "-m", "root"],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&repository)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }
        // The session runs somewhere that is not a repository at all — the
        // exact shape of the scratch-dir cwd that produced the issue.
        let elsewhere = data_dir.join("scratch");
        std::fs::create_dir(&elsewhere).unwrap();

        {
            let db = store::open(&data_dir.join("bridge.db")).unwrap();
            db.execute_batch(
                "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
                 INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                     VALUES('s',NULL,'codex','Codex','idle','reported');",
            )
            .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','main',?1,'idle','now')",
                params![repository.to_string_lossy()],
            )
            .unwrap();
            db.execute("UPDATE sessions SET workspace_id='w',cwd=?1 WHERE id='s'", params![elsewhere.to_string_lossy()]).unwrap();
        }

        let core = Arc::new(
            crate::BridgeCore::boot(crate::BootConfig {
                data_dir: data_dir.to_path_buf(),
                browser_extension_path: data_dir.join("no-extension"),
                events: None,
            })
            .unwrap(),
        );
        let divergence = crate::api::workspace_base_divergence(&core, "s", false)
            .expect("the workspace root is a repository and must measure");

        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repository)
            .output()
            .unwrap();
        assert_eq!(
            divergence.head.as_deref(),
            Some(String::from_utf8_lossy(&head.stdout).trim()),
            "the reading describes the workspace root, not the session cwd"
        );

        // The board must now hold the workspace root's reading — the same
        // directory the observer measures — instead of a failure.
        let db = core.db.lock().unwrap();
        let (status, payload, _, _) = cached(&db);
        assert_eq!(status, "ok");
        let stored: git::BaseBranchDivergence =
            serde_json::from_str(&payload.expect("payload")).unwrap();
        assert_eq!(stored, divergence);
    }

    #[test]
    fn a_workspace_is_due_when_it_has_never_been_observed_or_its_reading_has_aged() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        let now = at("2026-08-19T09:00:00+00:00");
        assert_eq!(
            due_workspaces(&db, now).unwrap(),
            vec![("w".to_owned(), "/tmp/w".to_owned())]
        );

        record_base_divergence(&db, "w", Ok(&divergence(31)), now).unwrap();
        assert!(
            due_workspaces(&db, at("2026-08-19T09:04:00+00:00")).unwrap().is_empty(),
            "a reading inside the refresh interval is not re-measured"
        );
        assert_eq!(
            due_workspaces(&db, at("2026-08-19T09:06:00+00:00")).unwrap().len(),
            1,
            "past the interval it is due again"
        );
    }

    #[test]
    fn a_workspace_with_no_session_is_never_observed() {
        let db = memory_db();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now');",
        )
        .unwrap();
        assert!(
            due_workspaces(&db, at("2026-08-19T09:00:00+00:00")).unwrap().is_empty(),
            "there would be nowhere to send a human, so there is nothing to measure"
        );
    }

    #[test]
    fn archiving_a_workspace_takes_its_observation_with_it() {
        let db = memory_db();
        seed(&db, "/tmp/w");
        record_base_divergence(&db, "w", Ok(&divergence(31)), at("2026-08-19T09:00:00+00:00"))
            .unwrap();
        crate::workspaces::archive_workspace_records(&db, "w", 0, || Ok(())).unwrap();
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM work_fact_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "the cache has no foreign key, so archiving must clean up after it");
    }
}
