//! The model jobs queue: Structure and Turn into tasks, run only on request.
//!
//! A leased queue like the title job's. A person's click (or the opt-in
//! auto-structure setting) enqueues one row; a maintenance loop claims it,
//! runs one tool-less hidden turn on the chosen harness with no database lock
//! held, and settles the row with its outcome, detail and spend. A row left
//! `running` by a crash goes back to `pending` when the store opens.

use bridge_protocol::messages::NoteJob;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::ids::new_id;
use super::service::{invalid, NotesService};
use super::{structure, tasks};
use crate::title_jobs::{self, Generation, GENERATION_HARNESSES};
use crate::BridgeError;

pub const MAX_ATTEMPTS: i64 = 3;
pub const STRUCTURE: &str = "structure";
pub const TASKS: &str = "tasks";

/// A project the model may suggest, keyed `p1..` in prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectChoice {
    pub id: String,
    pub label: String,
}

/// Where model passes run. The live implementation drives the adapter
/// registry; tests inject their own. Never called with the database locked.
pub trait ModelBackend {
    fn can_generate(&self, harness: &str) -> bool;
    /// The application's default chat harness and model, if one is set.
    fn default_choice(&self) -> Option<(String, Option<String>)>;
    fn projects(&self) -> Vec<ProjectChoice>;
    fn generate(&self, harness: &str, model: Option<&str>, instructions: &str, prompt: &str) -> Result<Generation, String>;
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn enqueue(db: &Connection, space: &str, page_id: &str, kind: &str, harness: Option<&str>, model: Option<&str>) -> Result<String, BridgeError> {
    if ![STRUCTURE, TASKS].contains(&kind) {
        return Err(invalid(format!("unknown model job {kind}")));
    }
    let id = new_id();
    let now = stamp(Utc::now());
    let clean = |value: Option<&str>| value.map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned);
    db.execute(
        "INSERT INTO model_jobs(id, space_id, page_id, kind, status, available_at, requested_harness, requested_model, created_at, updated_at)
         VALUES(?1,?2,?3,?4,'pending',?5,?6,?7,?5,?5)",
        params![id, space, page_id, kind, now, clean(harness), clean(model)],
    )?;
    Ok(id)
}

/// Return jobs a crash left running to the queue.
pub fn recover(db: &Connection) -> Result<usize, BridgeError> {
    Ok(db.execute(
        "UPDATE model_jobs SET status='pending', lease_owner=NULL, updated_at=?1 WHERE status='running'",
        params![stamp(Utc::now())],
    )?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedJob {
    pub id: String,
    pub page_id: String,
    pub kind: String,
    pub attempts: i64,
    pub requested_harness: Option<String>,
    pub requested_model: Option<String>,
}

pub fn claim(db: &Connection, owner: &str, now: DateTime<Utc>) -> Result<Option<ClaimedJob>, BridgeError> {
    let job = db
        .query_row(
            "SELECT id, page_id, kind, attempts, requested_harness, requested_model FROM model_jobs
             WHERE status='pending' AND available_at<=?1 ORDER BY available_at, id LIMIT 1",
            params![stamp(now)],
            |row| {
                Ok(ClaimedJob {
                    id: row.get(0)?,
                    page_id: row.get(1)?,
                    kind: row.get(2)?,
                    attempts: row.get(3)?,
                    requested_harness: row.get(4)?,
                    requested_model: row.get(5)?,
                })
            },
        )
        .optional()?;
    let Some(mut job) = job else { return Ok(None) };
    let claimed = db.execute(
        "UPDATE model_jobs SET status='running', lease_owner=?2, attempts=attempts+1, updated_at=?3 WHERE id=?1 AND status='pending'",
        params![job.id, owner, stamp(now)],
    )?;
    job.attempts += 1;
    Ok((claimed == 1).then_some(job))
}

struct Settle<'a> {
    status: &'a str,
    outcome: &'a str,
    detail: Option<String>,
    harness: Option<&'a str>,
    model: Option<&'a str>,
    generation: Option<&'a Generation>,
    retry_at: Option<DateTime<Utc>>,
}

fn settle(db: &Connection, id: &str, settle: Settle<'_>) -> Result<(), BridgeError> {
    let now = Utc::now();
    let (tokens, spend) = settle.generation.map(|generation| (generation.tokens, generation.spend_microusd)).unwrap_or((0, 0));
    db.execute(
        "UPDATE model_jobs SET status=?2, outcome=?3, detail=?4, harness=COALESCE(?5, harness), model=COALESCE(?6, model),
            tokens=tokens+?7, spend_microusd=spend_microusd+?8, lease_owner=NULL, available_at=?9, updated_at=?10
         WHERE id=?1",
        params![
            id,
            settle.status,
            settle.outcome,
            settle.detail,
            settle.harness,
            settle.model,
            tokens,
            spend,
            stamp(settle.retry_at.unwrap_or(now)),
            stamp(now)
        ],
    )?;
    Ok(())
}

/// Jobs a client should show: pending or running, plus anything that ended in
/// the last day.
pub fn recent(db: &Connection) -> Result<Vec<NoteJob>, BridgeError> {
    let since = stamp(Utc::now() - Duration::days(1));
    let mut statement = db.prepare(
        "SELECT id, page_id, kind, status, detail, COALESCE(harness, requested_harness), COALESCE(model, requested_model), created_at, updated_at
         FROM model_jobs WHERE status IN ('pending','running') OR updated_at>=?1 ORDER BY created_at DESC LIMIT 200",
    )?;
    let jobs = statement
        .query_map(params![since], |row| {
            Ok(NoteJob {
                id: row.get(0)?,
                page_id: row.get(1)?,
                kind: row.get(2)?,
                status: row.get(3)?,
                detail: row.get(4)?,
                harness: row.get(5)?,
                model: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(jobs)
}

/// The harness a job runs on: the one asked for, else the default chat
/// harness, else the first available. The second value is the default chat
/// model when it belongs to the chosen harness.
pub fn resolve_harness(backend: &dyn ModelBackend, requested: Option<&str>) -> Result<(String, Option<String>), String> {
    let default = backend.default_choice();
    let default_model = |harness: &str| default.as_ref().filter(|(name, _)| name == harness).and_then(|(_, model)| model.clone());
    if let Some(harness) = requested {
        if !GENERATION_HARNESSES.contains(&harness) {
            return Err(format!("{harness} cannot run note passes; choose Claude, Codex or OpenCode"));
        }
        if backend.can_generate(harness) {
            return Ok((harness.to_owned(), default_model(harness)));
        }
        return Err(format!("{harness} is not available right now; sign in or pick another harness"));
    }
    if let Some((harness, model)) = &default {
        if GENERATION_HARNESSES.contains(&harness.as_str()) && backend.can_generate(harness) {
            return Ok((harness.clone(), model.clone()));
        }
    }
    GENERATION_HARNESSES
        .iter()
        .find(|harness| backend.can_generate(harness))
        .map(|harness| (harness.to_string(), default_model(harness)))
        .ok_or_else(|| "No harness can run this: sign in to Claude, Codex or OpenCode.".to_owned())
}

/// The most used tags in the space, for the model to reuse.
fn top_tags(db: &Connection, space: &str) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT value, COUNT(*) AS uses FROM pages, json_each(pages.tags)
         WHERE pages.space_id=?1 AND pages.deleted_at IS NULL GROUP BY value ORDER BY uses DESC, value LIMIT 40",
    )?;
    let tags = statement.query_map(params![space], |row| row.get(0))?.collect::<Result<_, _>>()?;
    Ok(tags)
}

/// Claim one job and see it through. Returns the job id, or `None` when the
/// queue was empty.
pub fn drain_once(notes: &NotesService, backend: &dyn ModelBackend, owner: &str) -> Result<Option<String>, BridgeError> {
    let Some(job) = notes.with_db(|db| claim(db, owner, Utc::now()))? else {
        return Ok(None);
    };
    let fail = |detail: String, harness: Option<&str>, model: Option<&str>, generation: Option<&Generation>, retry: bool| {
        let retry_at = (retry && job.attempts < MAX_ATTEMPTS).then(|| Utc::now() + Duration::seconds(20 * job.attempts));
        notes.with_db(|db| {
            settle(db, &job.id, Settle { status: if retry_at.is_some() { "pending" } else { "failed" }, outcome: "failed", detail: Some(detail), harness, model, generation, retry_at })
        })
    };
    let Ok(page) = notes.read(&job.page_id) else {
        fail("the page was deleted".into(), None, None, None, false)?;
        return Ok(Some(job.id));
    };
    let (harness, default_model) = match resolve_harness(backend, job.requested_harness.as_deref()) {
        Ok(choice) => choice,
        Err(reason) => {
            fail(reason, job.requested_harness.as_deref(), None, None, false)?;
            return Ok(Some(job.id));
        }
    };
    let model = job.requested_model.clone().or_else(|| match job.kind.as_str() {
        STRUCTURE => title_jobs::title_model(&harness, default_model.as_deref()),
        _ => default_model.clone(),
    });
    let projects = backend.projects();
    let tags = notes.with_db(|db| top_tags(db, notes.space()))?;
    let (instructions, prompt) = match job.kind.as_str() {
        STRUCTURE => (structure::INSTRUCTIONS, structure::prompt(&page.markdown, &projects, &tags)),
        _ => (tasks::INSTRUCTIONS, tasks::prompt(&page.markdown, &projects)),
    };
    let generation = match backend.generate(&harness, model.as_deref(), instructions, &prompt) {
        Ok(generation) => generation,
        Err(detail) => {
            fail(detail, Some(&harness), model.as_deref(), None, true)?;
            return Ok(Some(job.id));
        }
    };
    let outcome = match job.kind.as_str() {
        STRUCTURE => structure::accept(notes, &page, &generation.text, &projects, &harness, model.as_deref()),
        _ => tasks::accept(notes, &page, &generation.text, &projects, &harness, model.as_deref()),
    };
    match outcome {
        Ok(detail) => notes.with_db(|db| {
            settle(db, &job.id, Settle { status: "done", outcome: "done", detail: Some(detail), harness: Some(&harness), model: model.as_deref(), generation: Some(&generation), retry_at: None })
        })?,
        Err(reason) => fail(reason, Some(&harness), model.as_deref(), Some(&generation), true)?,
    }
    Ok(Some(job.id))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::notes::service::tests::service;
    use crate::notes::service::PERSON;
    use std::sync::Mutex;

    /// A backend with canned replies, recording what it was asked.
    #[derive(Default)]
    pub(crate) struct FakeBackend {
        pub available: Vec<&'static str>,
        pub default: Option<(String, Option<String>)>,
        pub projects: Vec<ProjectChoice>,
        pub replies: Mutex<Vec<Result<String, String>>>,
        pub calls: Mutex<Vec<(String, Option<String>, String)>>,
    }

    impl FakeBackend {
        pub(crate) fn replying(replies: Vec<Result<String, String>>) -> Self {
            Self { available: vec!["claude", "codex"], replies: Mutex::new(replies), ..Default::default() }
        }
    }

    impl ModelBackend for FakeBackend {
        fn can_generate(&self, harness: &str) -> bool {
            self.available.contains(&harness)
        }
        fn default_choice(&self) -> Option<(String, Option<String>)> {
            self.default.clone()
        }
        fn projects(&self) -> Vec<ProjectChoice> {
            self.projects.clone()
        }
        fn generate(&self, harness: &str, model: Option<&str>, _instructions: &str, prompt: &str) -> Result<Generation, String> {
            self.calls.lock().unwrap().push((harness.to_owned(), model.map(str::to_owned), prompt.to_owned()));
            let reply = self.replies.lock().unwrap().remove(0)?;
            Ok(Generation { text: reply, tokens: 120, spend_microusd: 300 })
        }
    }

    fn job_row(notes: &NotesService, id: &str) -> (String, i64, Option<String>, Option<String>, i64) {
        notes
            .with_db(|db| Ok(db.query_row("SELECT status, attempts, detail, harness, spend_microusd FROM model_jobs WHERE id=?1", params![id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?))
            .unwrap()
    }

    #[test]
    fn running_jobs_return_to_pending_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let check: crate::notes::service::ProjectCheck = std::sync::Arc::new(|_: &str| false);
        let job = {
            let notes = NotesService::open(dir.path(), check.clone()).unwrap();
            let page = notes.capture("structure me", PERSON).unwrap();
            let job = notes.with_db(|db| enqueue(db, notes.space(), &page.id, STRUCTURE, None, None)).unwrap();
            notes.with_db(|db| claim(db, "crashed-owner", Utc::now())).unwrap().expect("claimed");
            assert_eq!(job_row(&notes, &job).0, "running");
            job
        };
        let reopened = NotesService::open(dir.path(), check).unwrap();
        let (status, attempts, ..) = job_row(&reopened, &job);
        assert_eq!((status.as_str(), attempts), ("pending", 1));
    }

    #[test]
    fn no_harness_fails_with_a_reason() {
        let (_dir, notes) = service();
        let page = notes.capture("structure me", PERSON).unwrap();
        let job = notes.with_db(|db| enqueue(db, notes.space(), &page.id, STRUCTURE, None, None)).unwrap();
        let backend = FakeBackend { available: vec![], ..Default::default() };
        drain_once(&notes, &backend, "owner").unwrap();
        let (status, _, detail, ..) = job_row(&notes, &job);
        assert_eq!(status, "failed");
        assert!(detail.unwrap().contains("sign in to Claude, Codex or OpenCode"));
        assert!(backend.calls.lock().unwrap().is_empty());

        let asked = notes.with_db(|db| enqueue(db, notes.space(), &page.id, TASKS, Some("opencode"), None)).unwrap();
        let backend = FakeBackend { available: vec!["claude"], ..Default::default() };
        drain_once(&notes, &backend, "owner").unwrap();
        let (status, _, detail, ..) = job_row(&notes, &asked);
        assert_eq!(status, "failed");
        assert!(detail.unwrap().contains("opencode is not available"));
        assert!(notes.read(&page.id).unwrap().tasks.is_empty());
    }

    #[test]
    fn auto_structure_is_off_by_default() {
        let (_dir, notes) = service();
        assert!(!notes.settings().unwrap().auto_structure);
        notes.capture("one", PERSON).unwrap();
        assert!(notes.with_db(recent).unwrap().is_empty());
        notes.save_settings(Some(true), None).unwrap();
        let page = notes.capture("two", PERSON).unwrap();
        let jobs = notes.with_db(recent).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!((jobs[0].kind.as_str(), jobs[0].page_id.as_str()), (STRUCTURE, page.id.as_str()));
    }

    #[test]
    fn harness_resolution_prefers_the_request_then_the_default() {
        let backend = FakeBackend { available: vec!["claude", "codex"], default: Some(("codex".into(), Some("gpt-5.6-terra".into()))), ..Default::default() };
        assert_eq!(resolve_harness(&backend, Some("claude")).unwrap(), ("claude".into(), None));
        assert_eq!(resolve_harness(&backend, None).unwrap(), ("codex".into(), Some("gpt-5.6-terra".into())));
        assert!(resolve_harness(&backend, Some("cursor")).unwrap_err().contains("cannot run"));
        let fallback = FakeBackend { available: vec!["opencode"], default: Some(("codex".into(), None)), ..Default::default() };
        assert_eq!(resolve_harness(&fallback, None).unwrap().0, "opencode");
    }

    #[test]
    fn a_failed_generation_retries_then_fails_and_records_spend() {
        let (_dir, notes) = service();
        let page = notes.capture("retry me", PERSON).unwrap();
        let job = notes.with_db(|db| enqueue(db, notes.space(), &page.id, STRUCTURE, Some("claude"), None)).unwrap();
        let backend = FakeBackend::replying(vec![Err("provider ended".into()), Ok("not json at all".into()), Err("deadline exceeded".into())]);
        for _ in 0..MAX_ATTEMPTS {
            notes.with_db(|db| Ok(db.execute("UPDATE model_jobs SET available_at='2000-01-01T00:00:00.000Z' WHERE id=?1", params![job])?)).unwrap();
            drain_once(&notes, &backend, "owner").unwrap();
        }
        let (status, attempts, detail, harness, spend) = job_row(&notes, &job);
        assert_eq!((status.as_str(), attempts, harness.as_deref()), ("failed", 3, Some("claude")));
        assert_eq!(detail.as_deref(), Some("deadline exceeded"));
        assert_eq!(spend, 300, "only the generation that returned text cost anything");
        assert!(notes.read(&page.id).unwrap().structured.is_none());
    }
}
