//! Pull-request reviews as stored background jobs.
//!
//! A review is a row in `review_jobs`, driven by Bridge's own runner with its
//! own slots, never a worker of a chat: starting one creates no session and
//! touches no conversation. See `state` for the job's lifecycle, `store` for
//! the durable guarantees, `runner` for slots, leases, retries and fallback,
//! and `steps` for the work a job does.

pub mod auto;
pub mod contract;
pub mod handoff;
pub mod merge;
pub mod overview;
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
    pub repo_host: String,
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
///
/// An asked-for pair is a second opinion and needs no setting: the Review
/// menu's Second opinion button sends one. The Second opinion setting only
/// decides whether the stored default (and so auto-review) is a pair, and
/// `settings::save` enforces that.
pub fn resolve_harnesses(core: &Arc<BridgeCore>, requested: Option<&[String]>) -> Result<Vec<String>, BridgeError> {
    let settings = {
        let db = lock_db(core);
        settings::load(&db)?
    };
    let asked = match requested {
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
            repo_host: head.repository.host,
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
    start_gated(core, params, head, &|_, _, _| true)?
        .ok_or_else(|| BridgeError::Invalid("The review was not started".into()))
}

/// A last check before a job is inserted, made under the store lock with the
/// settings as they are at that moment: nothing can change between it and
/// the insert. False means no job.
pub type StartGate<'a> = &'a dyn Fn(&rusqlite::Connection, &wire::ReviewJobSettings, store::PrKey<'_>) -> bool;

/// [`start_with`], with `gate` deciding at insertion time. `None` means the
/// gate refused.
pub fn start_gated(
    core: &Arc<BridgeCore>,
    params: &wire::StartReviewParams,
    head: PrHead,
    gate: StartGate<'_>,
) -> Result<Option<wire::ReviewJob>, BridgeError> {
    core.workspace_path(&params.workspace_id)?;
    let harnesses = resolve_harnesses(core, params.harnesses.as_deref())?;
    let trigger = params.trigger.unwrap_or_default();
    let (host, owner, name) = (head.repo_host.clone(), head.repo_owner.clone(), head.repo_name.clone());
    let pr = store::PrKey { host: &host, owner: &owner, name: &name, number: params.number };
    let (job, created, superseded) = {
        let db = lock_db(core);
        let settings = settings::load(&db)?;
        if !gate(&db, &settings, pr) {
            return Ok(None);
        }
        // A re-review starts at the commit the PR's last posted review is
        // on, when there is one and the PR has moved since.
        let incremental_from_sha = if settings.incremental {
            store::latest_posted_head(&db, pr)?.filter(|from| *from != head.head_sha)
        } else {
            None
        };
        let head_sha = head.head_sha.clone();
        let (job, created) = store::insert(
            &db,
            &store::NewJob {
                workspace_id: params.workspace_id.clone(),
                repo_host: Some(head.repo_host),
                repo_owner: head.repo_owner,
                repo_name: head.repo_name,
                number: params.number,
                head_sha: head.head_sha,
                base_sha: head.base_sha,
                incremental_from_sha,
                trigger,
                post_mode: settings.post_mode,
                harnesses,
                retry_of: None,
                publication_key: None,
            },
            Utc::now(),
        )?;
        // A push review replaces the queued push reviews of older heads.
        let superseded = if trigger == wire::ReviewTrigger::Push {
            store::supersede_queued_pushes(&db, pr, &head_sha, Utc::now())?
        } else {
            Vec::new()
        };
        (store::to_wire(&db, &job)?, created, superseded)
    };
    for id in &superseded {
        core.reviews.steps().cleanup(id);
        publish(core, id);
    }
    if created {
        core.events.publish(CoreEvent::ReviewJobUpdated { job: job.clone() });
        core.reviews.wake();
    }
    Ok(Some(job))
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
        summary: job.named_summary.clone().or_else(|| job.summary.clone()),
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
    let saved = {
        let db = lock_db(core);
        let before = settings::load(&db)?;
        let saved = settings::save(&db, next)?;
        // Under the same lock as the save: an auto-review start checks it
        // under that lock too, so none starts from the settings before.
        if (before.auto_review, before.skip_draft_prs) != (saved.auto_review, saved.skip_draft_prs) {
            core.reviews.auto.settings_changed();
        }
        saved
    };
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
    fn a_re_review_starts_at_the_last_posted_head_only_when_incremental_is_on() {
        let (_scratch, core) = core();
        let at = |sha: &str| PrHead { repo_host: "github.com".into(), repo_owner: "o".into(), repo_name: "r".into(), head_sha: sha.into(), base_sha: None };
        let request = wire::StartReviewParams { workspace_id: "w".into(), number: 3, harnesses: Some(vec!["claude".into()]), trigger: None };
        let first = start_with(&core, &request, at("aaa")).unwrap();
        assert_eq!(first.incremental_from_sha, None, "nothing posted yet");
        lock_db(&core)
            .execute(
                "UPDATE review_jobs SET status='posted', posted_head_sha=head_sha, finished_at='2026-11-24T10:00:00.000Z' WHERE id=?1",
                params![first.id],
            )
            .unwrap();

        let off = start_with(&core, &request, at("bbb")).unwrap();
        assert_eq!(off.incremental_from_sha, None, "off: a full review");
        cancel(&core, &off.id).unwrap();

        change(&core, |settings| settings.incremental = true);
        let on = start_with(&core, &request, at("ccc")).unwrap();
        assert_eq!(on.incremental_from_sha.as_deref(), Some("aaa"), "since the last posted head");
        let retried = {
            cancel(&core, &on.id).unwrap();
            retry(&core, &on.id).unwrap()
        };
        assert_eq!(retried.incremental_from_sha.as_deref(), Some("aaa"), "a retry keeps it");
        cancel(&core, &retried.id).unwrap();
        let same = start_with(&core, &request, at("aaa")).unwrap();
        assert_eq!(same.incremental_from_sha, None, "the posted head itself has nothing new");
    }

    #[test]
    fn an_asked_pair_is_a_second_opinion_without_the_setting() {
        let (_scratch, core) = core();
        let head = || PrHead { repo_host: "github.com".into(), repo_owner: "o".into(), repo_name: "r".into(), head_sha: "abc".into(), base_sha: None };
        let params = |harnesses: &[&str]| wire::StartReviewParams {
            workspace_id: "w".into(),
            number: 3,
            harnesses: Some(harnesses.iter().map(|harness| harness.to_string()).collect()),
            trigger: None,
        };
        // The Review menu's Second opinion button: Second opinion is off in
        // Settings, and the pair still runs as one job.
        assert!(!settings::load(&lock_db(&core)).unwrap().second_opinion);
        let both = start_with(&core, &params(&["claude", "codex"]), head()).unwrap();
        assert_eq!(both.harnesses, ["claude", "codex"]);
        let three = start_with(&core, &params(&["claude", "codex", "opencode"]), head()).unwrap_err().to_string();
        assert!(three.contains("at most 2"), "{three}");

        // A pair stored before the setting existed runs as it did: both review.
        change(&core, |_| {});
        lock_db(&core)
            .execute(
                "UPDATE configuration_entries SET payload=?1 WHERE kind='review_job_settings'",
                params![r#"{"defaultHarnesses":["claude","codex"]}"#],
            )
            .unwrap();
        assert_eq!(resolve_harnesses(&core, None).unwrap(), ["claude", "codex"]);
    }
}

