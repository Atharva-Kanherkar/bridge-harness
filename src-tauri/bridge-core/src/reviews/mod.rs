//! Pull-request reviews as stored background jobs.
//!
//! A review is a row in `review_jobs`, driven by Bridge's own runner with its
//! own slots, never a worker of a chat: starting one creates no session and
//! touches no conversation. See `state` for the job's lifecycle, `store` for
//! the durable guarantees, `runner` for slots, leases, retries and fallback,
//! and `steps` for the work a job does.

pub mod contract;
pub mod post;
pub mod prepare;
pub mod runner;
pub mod session;
pub mod settings;
pub mod state;
pub mod steps;
pub mod store;

use std::sync::Arc;

use bridge_protocol::messages as wire;
use chrono::Utc;

pub use runner::{start_review_maintenance, ReviewRuntime, Runner, RunnerConfig};

use crate::events::CoreEvent;
use crate::runtime::BridgeCore;
use crate::BridgeError;

/// Harnesses a review falls back to, in order, when the settings name none.
pub const DEFAULT_HARNESS_ORDER: [&str; 3] = ["claude", "codex", "opencode"];

/// What the PR looks like right now, read before a job is queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrHead {
    pub repo_owner: String,
    pub repo_name: String,
    pub head_sha: String,
    pub base_sha: Option<String>,
}

/// The store lock, through a poisoned mutex: one panicking job must not stop
/// every other review.
pub(crate) fn lock_db(core: &BridgeCore) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
    core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Tell clients where a job is now.
pub fn publish(core: &Arc<BridgeCore>, job_id: &str) {
    let job = {
        let db = lock_db(core);
        store::get_job(&db, job_id).ok().flatten().and_then(|job| store::to_wire(&db, &job).ok())
    };
    if let Some(job) = job {
        core.events.publish(CoreEvent::ReviewJobUpdated { job });
    }
}

/// The harnesses a job runs on: the ones asked for, else the settings'
/// default, else the first available of Claude, Codex and OpenCode.
pub fn resolve_harnesses(core: &Arc<BridgeCore>, requested: Option<&[String]>) -> Result<Vec<String>, BridgeError> {
    let settings = {
        let db = lock_db(core);
        settings::load(&db)?
    };
    let mut asked = match requested {
        Some([]) => return Err(BridgeError::Invalid("Pick a harness to review with".into())),
        Some(list) => settings::harnesses(list, "reviewer")?,
        None => settings::harnesses(&settings.default_harnesses, "default reviewer")?,
    };
    if asked.len() > settings::MAX_REVIEW_HARNESSES {
        return Err(BridgeError::Invalid(format!(
            "A review runs on at most {} harnesses",
            settings::MAX_REVIEW_HARNESSES
        )));
    }
    if asked.len() > 1 && !settings.second_opinion {
        if requested.is_some() {
            return Err(BridgeError::Invalid(
                "Turn on Second opinion in Settings to review with two harnesses".into(),
            ));
        }
        // Defaults saved while it was on: the first one reviews.
        asked.truncate(1);
    }
    if !asked.is_empty() {
        return Ok(asked);
    }
    DEFAULT_HARNESS_ORDER
        .into_iter()
        .find(|harness| core.adapter_registry.descriptor(harness).is_some_and(|descriptor| descriptor.available))
        .map(|harness| vec![harness.to_owned()])
        .ok_or_else(|| BridgeError::Invalid("No harness is available to review with".into()))
}

/// Queue a review of the PR as it is on GitHub now, or return the live job
/// that already reviews that head with those harnesses. The head is read
/// fresh, so the job reviews the PR's real head commit.
pub fn start(core: &Arc<BridgeCore>, params: &wire::StartReviewParams) -> Result<wire::ReviewJob, BridgeError> {
    let path = core.workspace_path(&params.workspace_id)?;
    // Refuse a bad harness list before spending a `gh` call on it.
    resolve_harnesses(core, params.harnesses.as_deref())?;
    let head = core
        .github_surface
        .pr_head(std::path::Path::new(&path), params.number)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    start_with(
        core,
        params,
        PrHead {
            repo_owner: head.repository.owner,
            repo_name: head.repository.name,
            head_sha: head.head_sha,
            base_sha: head.base_sha,
        },
    )
}

/// Queue a review of the PR at `head`, or return the live job that already
/// reviews that head with those harnesses.
pub fn start_with(core: &Arc<BridgeCore>, params: &wire::StartReviewParams, head: PrHead) -> Result<wire::ReviewJob, BridgeError> {
    core.workspace_path(&params.workspace_id)?;
    let harnesses = resolve_harnesses(core, params.harnesses.as_deref())?;
    let (job, created) = {
        let db = lock_db(core);
        let post_mode = settings::load(&db)?.post_mode;
        let (job, created) = store::insert(
            &db,
            &store::NewJob {
                workspace_id: params.workspace_id.clone(),
                repo_owner: head.repo_owner,
                repo_name: head.repo_name,
                number: params.number,
                head_sha: head.head_sha,
                base_sha: head.base_sha,
                incremental_from_sha: None,
                trigger: params.trigger.unwrap_or_default(),
                post_mode,
                harnesses,
                retry_of: None,
                publication_key: None,
            },
            Utc::now(),
        )?;
        (store::to_wire(&db, &job)?, created)
    };
    if created {
        core.events.publish(CoreEvent::ReviewJobUpdated { job: job.clone() });
        core.reviews.wake();
    }
    Ok(job)
}

pub fn list(core: &Arc<BridgeCore>, params: &wire::ListReviewsParams) -> Result<wire::ReviewJobList, BridgeError> {
    let db = lock_db(core);
    let jobs = store::list(&db, &params.workspace_id, params.number)?
        .iter()
        .map(|job| store::to_wire(&db, job))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(wire::ReviewJobList { jobs })
}

pub fn get(core: &Arc<BridgeCore>, job_id: &str) -> Result<wire::ReviewJobDetail, BridgeError> {
    let db = lock_db(core);
    let job = store::require_job(&db, job_id)?;
    Ok(wire::ReviewJobDetail {
        job: store::to_wire(&db, &job)?,
        passes: store::passes(&db, job_id)?,
        findings: store::findings(&db, job_id)?,
    })
}

/// Cancel a job. A job this process is driving stops through its cancel
/// token, and its runner cleans up; one nobody is driving is cleaned up here.
pub fn cancel(core: &Arc<BridgeCore>, job_id: &str) -> Result<wire::ReviewJob, BridgeError> {
    {
        let db = lock_db(core);
        store::cancel(&db, job_id, Utc::now())?;
    }
    if !core.reviews.trip(job_id) {
        core.reviews.steps().cleanup(job_id);
    }
    publish(core, job_id);
    let db = lock_db(core);
    store::to_wire(&db, &store::require_job(&db, job_id)?)
}

/// A new job with the same inputs as a failed, interrupted or cancelled one.
pub fn retry(core: &Arc<BridgeCore>, job_id: &str) -> Result<wire::ReviewJob, BridgeError> {
    let (job, created) = {
        let db = lock_db(core);
        let (job, created) = store::retry(&db, job_id, Utc::now())?;
        (store::to_wire(&db, &job)?, created)
    };
    if created {
        core.events.publish(CoreEvent::ReviewJobUpdated { job: job.clone() });
        core.reviews.wake();
    }
    Ok(job)
}

/// Store a person's decisions on a drafted job, all or none, and return
/// every finding of the job.
pub fn decide(core: &Arc<BridgeCore>, params: &wire::DecideReviewParams) -> Result<wire::ReviewFindingList, BridgeError> {
    let findings = {
        let db = lock_db(core);
        store::decide(&db, &params.job_id, &params.decisions, Utc::now())?;
        store::findings(&db, &params.job_id)?
    };
    publish(core, &params.job_id);
    Ok(wire::ReviewFindingList { findings })
}

/// Post a drafted job's kept and edited findings: the job moves to
/// `posting`, and the runner posts it as one `COMMENT` review.
pub fn post(core: &Arc<BridgeCore>, job_id: &str) -> Result<wire::ReviewJob, BridgeError> {
    let job = {
        let db = lock_db(core);
        let job = store::request_post(&db, job_id)?;
        store::to_wire(&db, &job)?
    };
    core.events.publish(CoreEvent::ReviewJobUpdated { job: job.clone() });
    core.reviews.wake();
    Ok(job)
}

pub fn job_settings(core: &Arc<BridgeCore>) -> Result<wire::ReviewJobSettings, BridgeError> {
    settings::load(&lock_db(core))
}

/// Save how review jobs run, and let the runner pick up a wider pool now.
pub fn save_job_settings(core: &Arc<BridgeCore>, next: &wire::ReviewJobSettings) -> Result<wire::ReviewJobSettings, BridgeError> {
    let saved = settings::save(&lock_db(core), next)?;
    core.reviews.wake();
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let scratch = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(scratch.path()));
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','p',?1,'now')", params![scratch.path().to_string_lossy()])
                .unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','w','main',?1,'idle','now')",
                params![scratch.path().to_string_lossy()],
            )
            .unwrap();
        }
        (scratch, core)
    }

    fn change(core: &Arc<BridgeCore>, edit: impl FnOnce(&mut wire::ReviewJobSettings)) {
        let mut next = job_settings(core).unwrap();
        edit(&mut next);
        save_job_settings(core, &next).unwrap();
    }

    #[test]
    fn two_harnesses_need_second_opinion() {
        let (_scratch, core) = core();
        let head = || PrHead { repo_owner: "o".into(), repo_name: "r".into(), head_sha: "abc".into(), base_sha: None };
        let params = |harnesses: &[&str]| wire::StartReviewParams {
            workspace_id: "w".into(),
            number: 3,
            harnesses: Some(harnesses.iter().map(|harness| harness.to_string()).collect()),
            trigger: None,
        };
        let refused = start_with(&core, &params(&["claude", "codex"]), head()).unwrap_err().to_string();
        assert!(refused.contains("Turn on Second opinion"), "{refused}");
        assert!(list(&core, &wire::ListReviewsParams { workspace_id: "w".into(), number: None }).unwrap().jobs.is_empty());

        change(&core, |settings| {
            settings.second_opinion = true;
            settings.default_harnesses = vec!["claude".into(), "codex".into()];
        });
        let both = start_with(&core, &params(&["claude", "codex"]), head()).unwrap();
        assert_eq!(both.harnesses, ["claude", "codex"]);

        // Turned off again: the saved pair falls back to its first.
        {
            let db = lock_db(&core);
            let mut stored = settings::load(&db).unwrap();
            stored.second_opinion = false;
            db.execute(
                "UPDATE configuration_entries SET payload=?1 WHERE kind='review_job_settings'",
                params![serde_json::to_string(&stored).unwrap()],
            )
            .unwrap();
        }
        assert_eq!(resolve_harnesses(&core, None).unwrap(), ["claude"]);
    }
}

