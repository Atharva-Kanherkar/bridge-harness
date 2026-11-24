//! Review jobs, their passes and their findings in SQLite.
//!
//! Three guarantees live here rather than in the runner, so they hold however
//! many runners share the database:
//!
//! * **Idempotent start.** A partial unique index allows one non-terminal job
//!   per `(workspace, PR, head SHA, harness set)`; starting the same review
//!   again returns that job.
//! * **One claim.** A claim is one compare-and-swap on `lease_owner IS NULL`,
//!   so two runners racing for a job produce one winner.
//! * **Leases outlive a crash.** A claimed job holds a lease its runner renews.
//!   An expired lease puts the job back where another runner can claim it,
//!   and a job that keeps losing its runner fails instead of looping.

use bridge_protocol::messages::{
    self as wire, ReviewFindingDecision, ReviewJobStatus as Status, ReviewPassStatus, ReviewPostMode, ReviewSeverity,
    ReviewSide, ReviewTrigger,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::state;
use super::steps::{DraftFinding, PassOutcome, PassUsage, PostedReview};
use crate::BridgeError;

/// Statuses a job can still leave. One non-terminal job per dedupe key.
pub const ACTIVE_SQL: &str = "('queued','preparing','reviewing','drafted','posting')";
/// Claims before a job that keeps losing its runner fails.
pub const MAX_ATTEMPTS: u32 = 3;
/// `reviews/list_reviews` returns at most this many jobs.
pub const LIST_LIMIT: i64 = 50;
/// How long a claim holds without a renewal.
pub const LEASE_SECONDS: i64 = 45;

pub const RESTARTED: &str = "Bridge restarted during the review";
pub const ABANDONED: &str = "Abandoned when the review was reclaimed";
pub const STOPPED_RESPONDING: &str = "The review stopped responding three times";
pub const RESTARTED_POSTING: &str = "Bridge restarted while posting; post again";
/// The longest body a person may write for a finding. GitHub takes 65,536
/// characters per comment; the label, title and footer need the rest.
pub const MAX_EDITED_BODY_CHARS: usize = 60_000;

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_jobs(
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            pr_number INTEGER NOT NULL,
            head_sha TEXT NOT NULL,
            base_sha TEXT,
            incremental_from_sha TEXT,
            trigger_kind TEXT NOT NULL CHECK(trigger_kind IN ('manual','push')),
            post_mode TEXT NOT NULL CHECK(post_mode IN ('draft','auto')),
            harnesses TEXT NOT NULL,
            dedupe_key TEXT NOT NULL,
            publication_key TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN
                ('queued','preparing','reviewing','drafted','posting','posted','failed','cancelled','interrupted')),
            error TEXT,
            summary TEXT,
            posted_review_id TEXT,
            posted_review_url TEXT,
            retry_of TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            lease_owner TEXT,
            lease_expires_at TEXT,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_review_jobs_pr ON review_jobs(workspace_id, pr_number, created_at);
        CREATE INDEX IF NOT EXISTS idx_review_jobs_status ON review_jobs(status, created_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_review_jobs_active_dedupe ON review_jobs(dedupe_key)
            WHERE status IN ('queued','preparing','reviewing','drafted','posting');
        CREATE TABLE IF NOT EXISTS review_passes(
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES review_jobs(id) ON DELETE CASCADE,
            slot INTEGER NOT NULL DEFAULT 0,
            attempt INTEGER NOT NULL DEFAULT 1,
            harness TEXT NOT NULL,
            model TEXT,
            effort TEXT,
            status TEXT NOT NULL CHECK(status IN ('running','done','failed','cancelled')),
            session_id TEXT,
            error TEXT,
            denied_requests INTEGER NOT NULL DEFAULT 0,
            tokens_in INTEGER NOT NULL DEFAULT 0,
            tokens_out INTEGER NOT NULL DEFAULT 0,
            cost_microusd INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_review_passes_job ON review_passes(job_id, slot, created_at);
        CREATE TABLE IF NOT EXISTS review_findings(
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES review_jobs(id) ON DELETE CASCADE,
            pass_id TEXT NOT NULL REFERENCES review_passes(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            line INTEGER NOT NULL,
            start_line INTEGER,
            side TEXT NOT NULL CHECK(side IN ('LEFT','RIGHT')),
            severity TEXT NOT NULL CHECK(severity IN ('critical','major','minor','nit')),
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0,
            fingerprint TEXT NOT NULL,
            agreed_by INTEGER NOT NULL DEFAULT 1,
            outside_diff INTEGER NOT NULL DEFAULT 0,
            decision TEXT NOT NULL DEFAULT 'pending' CHECK(decision IN ('pending','kept','dropped','edited')),
            edited_body TEXT,
            decided_at TEXT,
            github_comment_id TEXT,
            posted_at TEXT,
            fixed_in_sha TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_review_findings_job ON review_findings(job_id);
        CREATE INDEX IF NOT EXISTS idx_review_findings_fingerprint ON review_findings(fingerprint);",
    )?;
    Ok(())
}

/// Every timestamp this module writes. One format, so string order is time
/// order within these columns.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn lease_until(now: DateTime<Utc>) -> String {
    stamp(now + Duration::seconds(LEASE_SECONDS))
}

/// The identity idempotent start compares: harness order does not matter.
pub fn dedupe_key(workspace_id: &str, number: u64, head_sha: &str, harnesses: &[String]) -> String {
    let mut set: Vec<&str> = harnesses.iter().map(String::as_str).collect();
    set.sort_unstable();
    set.dedup();
    format!("{workspace_id}|{number}|{head_sha}|{}", set.join(","))
}

/// A stored job, as the runner and the API read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub id: String,
    pub workspace_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: u64,
    pub head_sha: String,
    pub base_sha: Option<String>,
    pub incremental_from_sha: Option<String>,
    pub trigger: ReviewTrigger,
    pub post_mode: ReviewPostMode,
    /// The harnesses asked for, in order: one per pass slot.
    pub harnesses: Vec<String>,
    /// What the posted review's marker names: this job's id, or the key of
    /// the job it retried, so a retry finds a review its original posted.
    pub publication_key: String,
    pub status: Status,
    pub error: Option<String>,
    pub summary: Option<String>,
    pub posted_review_id: Option<String>,
    pub posted_review_url: Option<String>,
    pub retry_of: Option<String>,
    pub attempts: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// The checkout the current attempt reads, once prepared.
    pub worktree_path: Option<String>,
}

const SELECT_JOB: &str = "SELECT id, workspace_id, repo_owner, repo_name, pr_number, head_sha, base_sha,
    incremental_from_sha, trigger_kind, post_mode, harnesses, status, error, summary, posted_review_id,
    posted_review_url, retry_of, attempts, lease_owner, lease_expires_at, created_at, started_at, finished_at,
    publication_key, worktree_path
    FROM review_jobs";

fn invalid_column(index: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        format!("unexpected review value `{text}`").into(),
    )
}

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    let trigger: String = row.get(8)?;
    let post_mode: String = row.get(9)?;
    let harnesses: String = row.get(10)?;
    let status: String = row.get(11)?;
    Ok(JobRow {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        repo_owner: row.get(2)?,
        repo_name: row.get(3)?,
        number: row.get::<_, i64>(4)?.max(0) as u64,
        head_sha: row.get(5)?,
        base_sha: row.get(6)?,
        incremental_from_sha: row.get(7)?,
        trigger: ReviewTrigger::parse(&trigger).ok_or_else(|| invalid_column(8, &trigger))?,
        post_mode: ReviewPostMode::parse(&post_mode).ok_or_else(|| invalid_column(9, &post_mode))?,
        harnesses: serde_json::from_str(&harnesses).map_err(|_| invalid_column(10, &harnesses))?,
        status: Status::parse(&status).ok_or_else(|| invalid_column(11, &status))?,
        error: row.get(12)?,
        summary: row.get(13)?,
        posted_review_id: row.get(14)?,
        posted_review_url: row.get(15)?,
        retry_of: row.get(16)?,
        attempts: row.get::<_, i64>(17)?.max(0) as u32,
        lease_owner: row.get(18)?,
        lease_expires_at: row.get(19)?,
        created_at: row.get(20)?,
        started_at: row.get(21)?,
        finished_at: row.get(22)?,
        publication_key: row.get(23)?,
        worktree_path: row.get(24)?,
    })
}

pub fn get_job(db: &Connection, id: &str) -> Result<Option<JobRow>, BridgeError> {
    Ok(db.query_row(&format!("{SELECT_JOB} WHERE id=?1"), params![id], read_job).optional()?)
}

pub fn require_job(db: &Connection, id: &str) -> Result<JobRow, BridgeError> {
    get_job(db, id)?.ok_or_else(|| BridgeError::Invalid(format!("No review job {id}")))
}

fn active_by_key(db: &Connection, key: &str) -> Result<Option<JobRow>, BridgeError> {
    Ok(db
        .query_row(&format!("{SELECT_JOB} WHERE dedupe_key=?1 AND status IN {ACTIVE_SQL}"), params![key], read_job)
        .optional()?)
}

/// What a new job is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub workspace_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: u64,
    pub head_sha: String,
    pub base_sha: Option<String>,
    pub incremental_from_sha: Option<String>,
    pub trigger: ReviewTrigger,
    pub post_mode: ReviewPostMode,
    pub harnesses: Vec<String>,
    pub retry_of: Option<String>,
    /// The publication key to carry; `None` makes the new job's id its key.
    pub publication_key: Option<String>,
}

/// Insert a queued job, or return the non-terminal job that already reviews
/// the same PR head with the same harnesses. The flag is true when a job was
/// created.
pub fn insert(db: &Connection, job: &NewJob, now: DateTime<Utc>) -> Result<(JobRow, bool), BridgeError> {
    if job.harnesses.is_empty() {
        return Err(BridgeError::Invalid("A review needs at least one harness".into()));
    }
    let key = dedupe_key(&job.workspace_id, job.number, &job.head_sha, &job.harnesses);
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if let Some(existing) = active_by_key(&transaction, &key)? {
        transaction.commit()?;
        return Ok((existing, false));
    }
    let id = Uuid::new_v4().to_string();
    let harnesses = serde_json::to_string(&job.harnesses).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    transaction.execute(
        "INSERT INTO review_jobs(id, workspace_id, repo_owner, repo_name, pr_number, head_sha, base_sha,
            incremental_from_sha, trigger_kind, post_mode, harnesses, dedupe_key, status, retry_of, created_at,
            publication_key)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'queued',?13,?14,COALESCE(?15,?1))",
        params![
            id,
            job.workspace_id,
            job.repo_owner,
            job.repo_name,
            job.number as i64,
            job.head_sha,
            job.base_sha,
            job.incremental_from_sha,
            job.trigger.as_str(),
            job.post_mode.as_str(),
            harnesses,
            key,
            job.retry_of,
            stamp(now),
            job.publication_key,
        ],
    )?;
    let created = require_job(&transaction, &id)?;
    transaction.commit()?;
    Ok((created, true))
}

/// Newest first, at most [`LIST_LIMIT`].
pub fn list(db: &Connection, workspace_id: &str, number: Option<u64>) -> Result<Vec<JobRow>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "{SELECT_JOB} WHERE workspace_id=?1 AND (?2 IS NULL OR pr_number=?2)
         ORDER BY created_at DESC, rowid DESC LIMIT ?3"
    ))?;
    let rows = statement
        .query_map(params![workspace_id, number.map(|number| number as i64), LIST_LIMIT], read_job)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Claim the oldest claimable job for `runner`: a queued job (it moves to
/// `preparing`) or a posting job nobody holds (it stays `posting`). One
/// compare-and-swap per candidate, so a racing runner that wins first is
/// simply skipped.
///
/// Every claim gets its own lease token, `<runner>/<uuid>`, returned as the
/// job's `lease_owner`. Every later write must present that token, so a
/// driver whose job was reclaimed, even by its own runner, writes nothing.
///
/// Claiming a queued job also settles what an abandoned attempt left behind:
/// passes still `running` are cancelled, and that attempt's findings and
/// summary are dropped, so the new attempt starts clean.
pub fn claim_next(db: &Connection, runner: &str, now: DateTime<Utc>) -> Result<Option<JobRow>, BridgeError> {
    claim(db, runner, now, true)
}

/// [`claim_next`], but only a posting job when `take_queued` is false: the
/// review checkout budget is full, and posting needs no checkout.
pub fn claim(db: &Connection, runner: &str, now: DateTime<Utc>, take_queued: bool) -> Result<Option<JobRow>, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let candidates: Vec<(String, String)> = transaction
        .prepare(
            "SELECT id, status FROM review_jobs
             WHERE (status='posting' OR (?1 AND status='queued')) AND lease_owner IS NULL
             ORDER BY created_at, rowid LIMIT 16",
        )?
        .query_map(params![take_queued], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (id, status) in candidates {
        let claim = format!("{runner}/{}", Uuid::new_v4());
        let changed = transaction.execute(
            "UPDATE review_jobs SET
                status=CASE WHEN status='queued' THEN 'preparing' ELSE status END,
                lease_owner=?2, lease_expires_at=?3, attempts=attempts+1, started_at=COALESCE(started_at, ?4)
             WHERE id=?1 AND lease_owner IS NULL AND status IN ('queued','posting')",
            params![id, claim, lease_until(now), stamp(now)],
        )?;
        if changed == 0 {
            continue;
        }
        if status == "queued" {
            transaction.execute(
                "UPDATE review_passes SET status='cancelled', error=?2, finished_at=?3 WHERE job_id=?1 AND status='running'",
                params![id, ABANDONED, stamp(now)],
            )?;
            transaction.execute("DELETE FROM review_findings WHERE job_id=?1", params![id])?;
            // The old attempt's checkout is not this attempt's: until this one
            // records its own, it counts as pending against the budget.
            transaction.execute("UPDATE review_jobs SET summary=NULL, worktree_path=NULL WHERE id=?1", params![id])?;
        }
        let job = get_job(&transaction, &id)?;
        transaction.commit()?;
        return Ok(job);
    }
    transaction.commit()?;
    Ok(None)
}

/// Whether `claim` still holds the job in `status`.
fn holds(db: &Connection, id: &str, claim: &str, status: Status) -> Result<bool, BridgeError> {
    let held: i64 = db.query_row(
        "SELECT COUNT(*) FROM review_jobs WHERE id=?1 AND lease_owner=?2 AND status=?3",
        params![id, claim, status.as_str()],
        |row| row.get(0),
    )?;
    Ok(held == 1)
}

/// Extend the lease. False means the job was cancelled, finished or
/// reclaimed: the holder must stop and write nothing.
pub fn renew(db: &Connection, id: &str, owner: &str, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    let changed = db.execute(
        &format!(
            "UPDATE review_jobs SET lease_expires_at=?3
             WHERE id=?1 AND lease_owner=?2 AND status IN {ACTIVE_SQL} AND lease_expires_at > ?4"
        ),
        params![id, owner, lease_until(now), stamp(now)],
    )?;
    Ok(changed == 1)
}

/// Move a held job between non-terminal statuses. `release` gives the lease
/// up (a draft waiting for a person holds none). False means the holder lost
/// the job.
pub fn advance(
    db: &Connection,
    id: &str,
    owner: &str,
    from: Status,
    to: Status,
    release: bool,
) -> Result<bool, BridgeError> {
    if to.is_terminal() {
        return Err(BridgeError::Invalid("advance moves between non-terminal statuses; use finish".into()));
    }
    state::check(from, to)?;
    let changed = db.execute(
        "UPDATE review_jobs SET status=?4,
            lease_owner=CASE WHEN ?5 THEN NULL ELSE lease_owner END,
            lease_expires_at=CASE WHEN ?5 THEN NULL ELSE lease_expires_at END
         WHERE id=?1 AND lease_owner=?2 AND status=?3",
        params![id, owner, from.as_str(), to.as_str(), release],
    )?;
    Ok(changed == 1)
}

/// Write a terminal status. With an owner, only that lease holder may; with
/// none (a person cancelling, startup recovery), any non-terminal job may.
/// False means the job had already moved on.
pub fn finish(
    db: &Connection,
    id: &str,
    owner: Option<&str>,
    to: Status,
    error: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    if !to.is_terminal() || to == Status::Posted {
        return Err(BridgeError::Invalid("finish writes failed, cancelled or interrupted; use mark_posted".into()));
    }
    let Some(job) = get_job(db, id)? else {
        return Ok(false);
    };
    if !state::allowed(job.status, to) {
        return Ok(false);
    }
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE review_jobs SET status=?4, error=?5, lease_owner=NULL, lease_expires_at=NULL, finished_at=?6
         WHERE id=?1 AND status=?2 AND (?3 IS NULL OR lease_owner=?3)",
        params![id, job.status.as_str(), owner, to.as_str(), error, stamp(now)],
    )?;
    if changed == 1 {
        // Nothing runs for a finished job: a pass still running ends with it.
        transaction.execute(
            "UPDATE review_passes SET status='cancelled', error=COALESCE(?2, error), finished_at=?3
             WHERE job_id=?1 AND status='running'",
            params![id, error, stamp(now)],
        )?;
    }
    transaction.commit()?;
    Ok(changed == 1)
}

/// `posting -> posted`, with the review GitHub holds, and each finding it
/// holds marked posted (with its comment id when it is inline), in one
/// transaction under `owner`.
pub fn mark_posted(db: &Connection, id: &str, owner: &str, posted: &PostedReview, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    state::check(Status::Posting, Status::Posted)?;
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE review_jobs SET status='posted', posted_review_id=?3, posted_review_url=?4, error=NULL,
            lease_owner=NULL, lease_expires_at=NULL, finished_at=?5, posted_head_sha=COALESCE(?6, head_sha)
         WHERE id=?1 AND status='posting' AND lease_owner=?2",
        params![id, owner, posted.review_id, posted.url, stamp(now), posted.commit_id],
    )?;
    if changed == 1 {
        for finding in &posted.findings {
            transaction.execute(
                "UPDATE review_findings SET posted_at=?3, github_comment_id=COALESCE(?4, github_comment_id)
                 WHERE id=?1 AND job_id=?2",
                params![finding.finding_id, id, stamp(now), finding.comment_id],
            )?;
        }
    }
    transaction.commit()?;
    Ok(changed == 1)
}

/// `posting -> drafted`: a post a person asked for failed, so the draft is
/// theirs again, with the reason, and they can post once more.
pub fn return_to_draft(db: &Connection, id: &str, owner: &str, error: &str) -> Result<bool, BridgeError> {
    state::check(Status::Posting, Status::Drafted)?;
    let changed = db.execute(
        "UPDATE review_jobs SET status='drafted', error=?3, lease_owner=NULL, lease_expires_at=NULL
         WHERE id=?1 AND status='posting' AND lease_owner=?2",
        params![id, owner, error],
    )?;
    Ok(changed == 1)
}

/// What [`set_head`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadUpdate {
    /// The job now reviews the new head, under the new dedupe key.
    Moved,
    /// Another live job already reviews the new head with the same harnesses.
    Superseded { by: String },
    /// The claim no longer holds the job.
    Lost,
}

/// The PR moved while the job waited: review what is there now. The head and
/// the dedupe key change together, so starting the same review of the new
/// head finds this job. When another live job already covers the new head,
/// nothing changes and the caller retires this one.
pub fn set_head(db: &Connection, id: &str, claim: &str, head_sha: &str) -> Result<HeadUpdate, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    let Some(job) = get_job(&transaction, id)? else {
        return Ok(HeadUpdate::Lost);
    };
    if job.lease_owner.as_deref() != Some(claim) {
        return Ok(HeadUpdate::Lost);
    }
    let key = dedupe_key(&job.workspace_id, job.number, head_sha, &job.harnesses);
    if let Some(other) = active_by_key(&transaction, &key)?.filter(|other| other.id != id) {
        return Ok(HeadUpdate::Superseded { by: other.id });
    }
    transaction.execute(
        "UPDATE review_jobs SET head_sha=?3, dedupe_key=?4 WHERE id=?1 AND lease_owner=?2",
        params![id, claim, head_sha, key],
    )?;
    transaction.commit()?;
    Ok(HeadUpdate::Moved)
}

/// Record the checkout the current attempt prepared, under `claim`. False
/// means the claim no longer holds the job.
pub fn set_worktree(db: &Connection, id: &str, claim: &str, path: &str) -> Result<bool, BridgeError> {
    let changed = db.execute(
        "UPDATE review_jobs SET worktree_path=?3 WHERE id=?1 AND lease_owner=?2 AND status='preparing'",
        params![id, claim, path],
    )?;
    Ok(changed == 1)
}

/// Give up a reservation `set_worktree` made, when cutting the checkout
/// failed.
pub fn clear_worktree(db: &Connection, id: &str, claim: &str) -> Result<(), BridgeError> {
    db.execute("UPDATE review_jobs SET worktree_path=NULL WHERE id=?1 AND lease_owner=?2", params![id, claim])?;
    Ok(())
}

/// Checkouts reserved or in use by jobs preparing or reviewing now. A
/// reservation counts against the budget before its directory exists.
pub fn reserved_worktrees(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT worktree_path FROM review_jobs WHERE status IN ('preparing','reviewing') AND worktree_path IS NOT NULL",
    )?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}

/// The current attempt of every live job, for sweeping what earlier attempts
/// left.
pub fn live_attempts(db: &Connection) -> Result<Vec<(String, u32)>, BridgeError> {
    let mut statement = db.prepare(&format!("SELECT id, attempts FROM review_jobs WHERE status IN {ACTIVE_SQL}"))?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u32)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The checkout of every job that is still live: what the reaper must keep.
pub fn live_worktrees(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT worktree_path FROM review_jobs WHERE status IN {ACTIVE_SQL} AND worktree_path IS NOT NULL"
    ))?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}

/// Ids of jobs that are still live.
pub fn live_job_ids(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(&format!("SELECT id FROM review_jobs WHERE status IN {ACTIVE_SQL}"))?;
    let ids = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// Claimed jobs that will need a checkout and have none recorded yet.
pub fn claimed_without_worktree(db: &Connection) -> Result<usize, BridgeError> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM review_jobs WHERE status='preparing' AND worktree_path IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

/// Which live job, if any, reads the checkout at `path`.
pub fn live_job_for_worktree(db: &Connection, path: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            &format!("SELECT id FROM review_jobs WHERE worktree_path=?1 AND status IN {ACTIVE_SQL} LIMIT 1"),
            params![path],
            |row| row.get(0),
        )
        .optional()?)
}

fn set_summary(db: &Connection, id: &str, summary: &str) -> Result<(), BridgeError> {
    db.execute("UPDATE review_jobs SET summary=?2 WHERE id=?1", params![id, summary])?;
    Ok(())
}

/// Cancel a non-terminal job. Its runner, if any, learns through its cancel
/// token and its next renewal.
pub fn cancel(db: &Connection, id: &str, now: DateTime<Utc>) -> Result<JobRow, BridgeError> {
    let job = require_job(db, id)?;
    if job.status.is_terminal() {
        return Err(BridgeError::Invalid(format!("This review is already {}", job.status.as_str())));
    }
    if !finish(db, id, None, Status::Cancelled, None, now)? {
        let now_status = require_job(db, id)?.status;
        return Err(BridgeError::Invalid(format!("This review is already {}", now_status.as_str())));
    }
    require_job(db, id)
}

/// Settle every in-flight job whose lease expired. Preparing and reviewing
/// go back to the queue; a posting job keeps its status and loses its lease
/// (posting is idempotent by marker); a held auto-mode draft moves on to
/// posting. A job that already used [`MAX_ATTEMPTS`] claims fails. One
/// compare-and-swap per row: a holder that renewed first is left alone.
pub fn reclaim_expired(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let expired: Vec<(String, String, u32, String, String)> = db
        .prepare(
            "SELECT id, status, attempts, lease_expires_at, post_mode FROM review_jobs
             WHERE status IN ('preparing','reviewing','drafted','posting')
               AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?1",
        )?
        .query_map(params![stamp(now)], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)?.max(0) as u32, row.get(3)?, row.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut changed = Vec::new();
    for (id, status, attempts, expiry, post_mode) in expired {
        let rows = if attempts >= MAX_ATTEMPTS {
            let rows = db.execute(
                "UPDATE review_jobs SET status='failed', error=?4, lease_owner=NULL, lease_expires_at=NULL,
                    finished_at=?5
                 WHERE id=?1 AND status=?2 AND lease_expires_at=?3",
                params![id, status, expiry, STOPPED_RESPONDING, stamp(now)],
            )?;
            if rows == 1 {
                db.execute(
                    "UPDATE review_passes SET status='cancelled', error=?2, finished_at=?3 WHERE job_id=?1 AND status='running'",
                    params![id, STOPPED_RESPONDING, stamp(now)],
                )?;
            }
            rows
        } else {
            let next = match status.as_str() {
                "preparing" | "reviewing" => "queued",
                "drafted" if post_mode == "auto" => "posting",
                other => other,
            };
            db.execute(
                "UPDATE review_jobs SET status=?4, lease_owner=NULL, lease_expires_at=NULL
                 WHERE id=?1 AND status=?2 AND lease_expires_at=?3",
                params![id, status, expiry, next],
            )?
        };
        if rows == 1 {
            changed.push(id);
        }
    }
    Ok(changed)
}

/// At boot: nothing in flight survived the restart. Every job a runner held,
/// and every post waiting for one, becomes `interrupted`, except a post a
/// person asked for: that goes back to their draft, triage intact, to post
/// again (the marker check keeps the next post from doubling one that got
/// through).
pub fn interrupt_in_flight(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let ids: Vec<(String, String, String)> = db
        .prepare(
            "SELECT id, status, post_mode FROM review_jobs
             WHERE status IN ('preparing','reviewing','posting') OR (status='drafted' AND lease_owner IS NOT NULL)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut changed = Vec::new();
    for (id, status, post_mode) in ids {
        let settled = if status == "posting" && post_mode == ReviewPostMode::Draft.as_str() {
            state::check(Status::Posting, Status::Drafted)?;
            db.execute(
                "UPDATE review_jobs SET status='drafted', error=?2, lease_owner=NULL, lease_expires_at=NULL
                 WHERE id=?1 AND status='posting'",
                params![id, RESTARTED_POSTING],
            )? == 1
        } else {
            finish(db, &id, None, Status::Interrupted, Some(RESTARTED), now)?
        };
        if settled {
            changed.push(id);
        }
    }
    Ok(changed)
}

// --- triage -------------------------------------------------------------------

/// A drafted job waiting for a person: no runner holds it.
fn waiting_draft(db: &Connection, job_id: &str, action: &str) -> Result<JobRow, BridgeError> {
    let job = require_job(db, job_id)?;
    if job.status != Status::Drafted || job.lease_owner.is_some() {
        return Err(BridgeError::Invalid(format!(
            "Only a drafted review waiting for you can be {action}; this one is {}",
            job.status.as_str()
        )));
    }
    Ok(job)
}

/// Store a person's decisions on a drafted job, all or none.
pub fn decide(
    db: &Connection,
    job_id: &str,
    decisions: &[wire::ReviewFindingDecisionInput],
    now: DateTime<Utc>,
) -> Result<(), BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    waiting_draft(&transaction, job_id, "triaged")?;
    let mut seen = std::collections::HashSet::new();
    for decision in decisions {
        if !seen.insert(decision.finding_id.as_str()) {
            return Err(BridgeError::Invalid(format!("Finding {} is decided twice", decision.finding_id)));
        }
        let edited_body = match (decision.decision, decision.edited_body.as_deref()) {
            (ReviewFindingDecision::Pending, _) => {
                return Err(BridgeError::Invalid("A decision is kept, dropped or edited".into()))
            }
            (ReviewFindingDecision::Edited, Some(body)) if !body.trim().is_empty() => {
                if body.chars().count() > MAX_EDITED_BODY_CHARS {
                    return Err(BridgeError::Invalid(format!(
                        "An edited finding is at most {MAX_EDITED_BODY_CHARS} characters"
                    )));
                }
                Some(body.trim().to_owned())
            }
            (ReviewFindingDecision::Edited, _) => {
                return Err(BridgeError::Invalid("An edited finding needs its new body".into()))
            }
            (_, Some(_)) => {
                return Err(BridgeError::Invalid("Only an edited finding carries a new body".into()))
            }
            (_, None) => None,
        };
        let changed = transaction.execute(
            "UPDATE review_findings SET decision=?3, edited_body=?4, decided_at=?5 WHERE id=?1 AND job_id=?2",
            params![decision.finding_id, job_id, decision.decision.as_str(), edited_body, stamp(now)],
        )?;
        if changed != 1 {
            return Err(BridgeError::Invalid(format!("This review has no finding {}", decision.finding_id)));
        }
    }
    transaction.commit()?;
    Ok(())
}

/// `drafted -> posting` for a person's post: allowed when at least one
/// finding is kept or edited, or the job has none (a summary-only review).
/// The runner claims it from there.
pub fn request_post(db: &Connection, job_id: &str) -> Result<JobRow, BridgeError> {
    state::check(Status::Drafted, Status::Posting)?;
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    waiting_draft(&transaction, job_id, "posted")?;
    let (total, chosen): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(decision IN ('kept','edited')), 0) FROM review_findings WHERE job_id=?1",
        params![job_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if total > 0 && chosen == 0 {
        return Err(BridgeError::Invalid("Keep or edit at least one finding to post this review".into()));
    }
    transaction.execute(
        "UPDATE review_jobs SET status='posting', error=NULL WHERE id=?1 AND status='drafted' AND lease_owner IS NULL",
        params![job_id],
    )?;
    let job = require_job(&transaction, job_id)?;
    transaction.commit()?;
    Ok(job)
}

/// A new job with the same inputs, linked to the one it retries. Idempotent
/// like start.
pub fn retry(db: &Connection, id: &str, now: DateTime<Utc>) -> Result<(JobRow, bool), BridgeError> {
    let job = require_job(db, id)?;
    if !matches!(job.status, Status::Failed | Status::Interrupted | Status::Cancelled) {
        return Err(BridgeError::Invalid(
            "Only a failed, interrupted or cancelled review can be retried".into(),
        ));
    }
    insert(
        db,
        &NewJob {
            workspace_id: job.workspace_id,
            repo_owner: job.repo_owner,
            repo_name: job.repo_name,
            number: job.number,
            head_sha: job.head_sha,
            base_sha: job.base_sha,
            incremental_from_sha: job.incremental_from_sha,
            trigger: job.trigger,
            post_mode: job.post_mode,
            harnesses: job.harnesses,
            retry_of: Some(job.id),
            publication_key: Some(job.publication_key),
        },
        now,
    )
}

// --- passes -------------------------------------------------------------------
//
// The runner only uses the claim-checked writes (`begin_pass`, `complete_pass`,
// `end_pass`, `finish_review`): each runs in one transaction that first checks
// the claim still holds the job. The unchecked primitives below them seed
// fixtures.

/// Start a pass under `claim`. `None` means the claim no longer holds the job.
pub fn begin_pass(
    db: &Connection,
    job_id: &str,
    claim: &str,
    slot: u32,
    harness: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if !holds(&transaction, job_id, claim, Status::Reviewing)? {
        return Ok(None);
    }
    let attempt = require_job(&transaction, job_id)?.attempts.max(1);
    let id = insert_pass(&transaction, job_id, slot, attempt, harness, now)?;
    transaction.commit()?;
    Ok(Some(id))
}

/// Store a finished pass and its findings, together, under `claim`. False
/// means the claim no longer holds the job and nothing was written.
pub fn complete_pass(
    db: &Connection,
    job_id: &str,
    claim: &str,
    pass_id: &str,
    outcome: &PassOutcome,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if !holds(&transaction, job_id, claim, Status::Reviewing)? {
        return Ok(false);
    }
    finish_pass(&transaction, pass_id, ReviewPassStatus::Done, None, Some(outcome), now)?;
    insert_findings(&transaction, job_id, pass_id, &outcome.findings, now)?;
    mark_fixed(&transaction, job_id, &outcome.fixed)?;
    transaction.commit()?;
    Ok(true)
}

/// Settle a pass that did not finish (failed or cancelled) under `claim`.
pub fn end_pass(
    db: &Connection,
    job_id: &str,
    claim: &str,
    pass_id: &str,
    status: ReviewPassStatus,
    error: Option<&str>,
    usage: PassUsage,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if !holds(&transaction, job_id, claim, Status::Reviewing)? {
        return Ok(false);
    }
    let spent = PassOutcome {
        tokens_in: usage.tokens_in,
        tokens_out: usage.tokens_out,
        cost_microusd: usage.cost_microusd,
        denied_requests: usage.denied_requests,
        ..PassOutcome::default()
    };
    finish_pass(&transaction, pass_id, status, error, Some(&spent), now)?;
    transaction.commit()?;
    Ok(true)
}

/// `reviewing -> drafted` with the summary, in one transaction under
/// `claim`. `release` gives the lease up (a draft waiting for a person holds
/// none).
pub fn finish_review(db: &Connection, job_id: &str, claim: &str, summary: &str, release: bool) -> Result<bool, BridgeError> {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if !holds(&transaction, job_id, claim, Status::Reviewing)? {
        return Ok(false);
    }
    set_summary(&transaction, job_id, summary)?;
    let moved = advance(&transaction, job_id, claim, Status::Reviewing, Status::Drafted, release)?;
    transaction.commit()?;
    Ok(moved)
}

/// Unchecked: fixtures only. The runner uses [`begin_pass`].
pub fn insert_pass(
    db: &Connection,
    job_id: &str,
    slot: u32,
    attempt: u32,
    harness: &str,
    now: DateTime<Utc>,
) -> Result<String, BridgeError> {
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO review_passes(id, job_id, slot, attempt, harness, status, created_at, started_at)
         VALUES(?1,?2,?3,?4,?5,'running',?6,?6)",
        params![id, job_id, slot, attempt, harness, stamp(now)],
    )?;
    Ok(id)
}

/// Unchecked: fixtures only. The runner uses [`complete_pass`] or [`end_pass`].
pub fn finish_pass(
    db: &Connection,
    pass_id: &str,
    status: ReviewPassStatus,
    error: Option<&str>,
    outcome: Option<&PassOutcome>,
    now: DateTime<Utc>,
) -> Result<(), BridgeError> {
    let empty = PassOutcome::default();
    let outcome = outcome.unwrap_or(&empty);
    db.execute(
        "UPDATE review_passes SET status=?2, error=?3, session_id=COALESCE(?4, session_id),
            model=COALESCE(?5, model), effort=COALESCE(?6, effort), tokens_in=?7, tokens_out=?8,
            cost_microusd=?9, denied_requests=?10, finished_at=?11
         WHERE id=?1",
        params![
            pass_id,
            status.as_str(),
            error,
            outcome.session_id,
            outcome.model,
            outcome.effort,
            outcome.tokens_in as i64,
            outcome.tokens_out as i64,
            outcome.cost_microusd as i64,
            outcome.denied_requests,
            stamp(now),
        ],
    )?;
    Ok(())
}

pub fn passes(db: &Connection, job_id: &str) -> Result<Vec<wire::ReviewPass>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, job_id, slot, harness, model, effort, status, session_id, error, denied_requests,
            tokens_in, tokens_out, cost_microusd, started_at, finished_at, attempt
         FROM review_passes WHERE job_id=?1 ORDER BY attempt, slot, created_at, rowid",
    )?;
    let rows = statement
        .query_map(params![job_id], |row| {
            let status: String = row.get(6)?;
            Ok(wire::ReviewPass {
                id: row.get(0)?,
                job_id: row.get(1)?,
                slot: row.get::<_, i64>(2)?.max(0) as u32,
                attempt: row.get::<_, i64>(15)?.max(1) as u32,
                harness: row.get(3)?,
                model: row.get(4)?,
                effort: row.get(5)?,
                status: ReviewPassStatus::parse(&status).ok_or_else(|| invalid_column(6, &status))?,
                session_id: row.get(7)?,
                error: row.get(8)?,
                denied_requests: row.get::<_, i64>(9)?.max(0) as u32,
                tokens_in: row.get::<_, i64>(10)?.max(0) as u64,
                tokens_out: row.get::<_, i64>(11)?.max(0) as u64,
                cost_microusd: row.get::<_, i64>(12)?.max(0) as u64,
                started_at: row.get(13)?,
                finished_at: row.get(14)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// --- findings -----------------------------------------------------------------

/// Unchecked: fixtures only. The runner uses [`complete_pass`].
pub fn insert_findings(
    db: &Connection,
    job_id: &str,
    pass_id: &str,
    findings: &[DraftFinding],
    now: DateTime<Utc>,
) -> Result<usize, BridgeError> {
    let mut statement = db.prepare(
        "INSERT INTO review_findings(id, job_id, pass_id, path, line, start_line, side, severity, title, body,
            confidence, fingerprint, outside_diff, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
    )?;
    for finding in findings {
        statement.execute(params![
            Uuid::new_v4().to_string(),
            job_id,
            pass_id,
            finding.path,
            finding.line,
            finding.start_line,
            finding.side.as_str(),
            finding.severity.as_str(),
            finding.title,
            finding.body,
            finding.confidence.clamp(0.0, 1.0),
            finding.fingerprint,
            finding.outside_diff,
            stamp(now),
        ])?;
    }
    Ok(findings.len())
}

const SEVERITY_RANK: &str =
    "CASE severity WHEN 'critical' THEN 0 WHEN 'major' THEN 1 WHEN 'minor' THEN 2 ELSE 3 END";

pub fn findings(db: &Connection, job_id: &str) -> Result<Vec<wire::ReviewFinding>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT id, job_id, pass_id, path, line, start_line, side, severity, title, body, confidence, fingerprint,
            agreed_by, outside_diff, decision, edited_body, github_comment_id, posted_at, decided_at, fixed_in_sha
         FROM review_findings WHERE job_id=?1
         ORDER BY agreed_by DESC, {SEVERITY_RANK}, confidence DESC, rowid"
    ))?;
    let rows = statement
        .query_map(params![job_id], |row| {
            let side: String = row.get(6)?;
            let severity: String = row.get(7)?;
            let decision: String = row.get(14)?;
            Ok(wire::ReviewFinding {
                id: row.get(0)?,
                job_id: row.get(1)?,
                pass_id: row.get(2)?,
                path: row.get(3)?,
                line: row.get::<_, i64>(4)?.max(0) as u32,
                start_line: row.get::<_, Option<i64>>(5)?.map(|line| line.max(0) as u32),
                side: ReviewSide::parse(&side).ok_or_else(|| invalid_column(6, &side))?,
                severity: ReviewSeverity::parse(&severity).ok_or_else(|| invalid_column(7, &severity))?,
                title: row.get(8)?,
                body: row.get(9)?,
                confidence: row.get(10)?,
                fingerprint: row.get(11)?,
                agreed_by: row.get::<_, i64>(12)?.max(1) as u32,
                outside_diff: row.get(13)?,
                decision: ReviewFindingDecision::parse(&decision).ok_or_else(|| invalid_column(14, &decision))?,
                edited_body: row.get(15)?,
                decided_at: row.get(18)?,
                github_comment_id: row.get(16)?,
                posted_at: row.get(17)?,
                fixed_in_sha: row.get(19)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The harness whose pass found each finding of a job.
pub fn finding_harnesses(db: &Connection, job_id: &str) -> Result<std::collections::HashMap<String, String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT f.id, p.harness FROM review_findings f JOIN review_passes p ON p.id=f.pass_id WHERE f.job_id=?1",
    )?;
    let rows = statement.query_map(params![job_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(rows)
}

/// The harnesses whose passes finished, in slot order, each once.
pub fn finished_harnesses(db: &Connection, job_id: &str) -> Result<Vec<String>, BridgeError> {
    let mut statement =
        db.prepare("SELECT harness FROM review_passes WHERE job_id=?1 AND status='done' ORDER BY slot, created_at, rowid")?;
    let all: Vec<String> = statement.query_map(params![job_id], |row| row.get(0))?.collect::<Result<_, _>>()?;
    let mut seen = std::collections::HashSet::new();
    Ok(all.into_iter().filter(|harness| seen.insert(harness.clone())).collect())
}

fn finding_counts(db: &Connection, job_id: &str) -> Result<wire::ReviewFindingCounts, BridgeError> {
    let mut counts = wire::ReviewFindingCounts::default();
    let mut statement = db.prepare("SELECT severity, COUNT(*) FROM review_findings WHERE job_id=?1 GROUP BY severity")?;
    let rows = statement.query_map(params![job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (severity, count) = row?;
        let count = count.max(0) as u32;
        counts.total += count;
        match ReviewSeverity::parse(&severity) {
            Some(ReviewSeverity::Critical) => counts.critical += count,
            Some(ReviewSeverity::Major) => counts.major += count,
            Some(ReviewSeverity::Minor) => counts.minor += count,
            Some(ReviewSeverity::Nit) => counts.nit += count,
            None => {}
        }
    }
    Ok(counts)
}

/// The harness each slot actually ran on: its latest pass, else the one
/// asked for.
fn slot_harnesses(db: &Connection, job: &JobRow) -> Result<Vec<String>, BridgeError> {
    let mut harnesses = job.harnesses.clone();
    for (slot, harness) in harnesses.iter_mut().enumerate() {
        let latest: Option<String> = db
            .query_row(
                "SELECT harness FROM review_passes WHERE job_id=?1 AND slot=?2 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![job.id, slot as i64],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(latest) = latest {
            *harness = latest;
        }
    }
    Ok(harnesses)
}

/// A pull request by what it is on GitHub, not by which workspace looked at
/// it: two workspaces of one repository share its history, and a workspace
/// pointed at another repository shares none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrKey<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub number: u64,
}

impl<'a> PrKey<'a> {
    pub fn of(job: &'a JobRow) -> Self {
        Self { owner: &job.repo_owner, name: &job.repo_name, number: job.number }
    }
}

/// The SQL that matches a [`PrKey`] at `?1..?3`, on `review_jobs` as `table`.
fn same_pr(table: &str) -> String {
    format!("{table}.repo_owner=?1 COLLATE NOCASE AND {table}.repo_name=?2 COLLATE NOCASE AND {table}.pr_number=?3")
}

/// The commit the PR's latest posted review is on: where a re-review starts.
/// That is the commit GitHub holds the review on, which a reconciled retry
/// can leave older than the job's own head.
pub fn latest_posted_head(db: &Connection, pr: PrKey<'_>) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            &format!(
                "SELECT COALESCE(posted_head_sha, head_sha) FROM review_jobs WHERE {} AND status='posted'
                 ORDER BY COALESCE(finished_at, created_at) DESC, rowid DESC LIMIT 1",
                same_pr("review_jobs")
            ),
            params![pr.owner, pr.name, pr.number as i64],
            |row| row.get(0),
        )
        .optional()?)
}

/// Workspaces that have review jobs: auto-review may watch them.
pub fn reviewed_workspaces(db: &Connection) -> Result<Vec<String>, BridgeError> {
    Ok(db
        .prepare("SELECT DISTINCT workspace_id FROM review_jobs ORDER BY workspace_id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

/// Each PR's latest review in a repository, from any workspace: what
/// started it and the head it covered.
pub fn latest_jobs(db: &Connection, owner: &str, name: &str) -> Result<std::collections::HashMap<u64, (ReviewTrigger, String)>, BridgeError> {
    let mut latest = std::collections::HashMap::new();
    let mut statement = db.prepare(
        "SELECT pr_number, trigger_kind, head_sha FROM review_jobs
         WHERE repo_owner=?1 COLLATE NOCASE AND repo_name=?2 COLLATE NOCASE ORDER BY created_at, rowid",
    )?;
    let rows = statement.query_map(params![owner, name], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (number, trigger, head) = row?;
        if let Some(trigger) = ReviewTrigger::parse(&trigger) {
            latest.insert(number.max(0) as u64, (trigger, head));
        }
    }
    Ok(latest)
}

/// Whether the PR already has a review of `head_sha`, from any workspace,
/// that did not fail or get cancelled: a push review of it would say the
/// same again.
pub fn has_review_of_head(db: &Connection, pr: PrKey<'_>, head_sha: &str) -> Result<bool, BridgeError> {
    Ok(db
        .query_row(
            &format!(
                "SELECT 1 FROM review_jobs WHERE {} AND head_sha=?4
                 AND status NOT IN ('failed','cancelled','interrupted') LIMIT 1",
                same_pr("review_jobs")
            ),
            params![pr.owner, pr.name, pr.number as i64, head_sha],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Why a queued push review was cancelled.
pub const SUPERSEDED_BY_PUSH: &str = "Superseded by a newer push";

/// Cancel the PR's queued push reviews of other heads: a newer push replaced
/// them. A person's own reviews are theirs and stay. Returns the ids
/// cancelled.
pub fn supersede_queued_pushes(db: &Connection, pr: PrKey<'_>, head_sha: &str, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let queued: Vec<String> = db
        .prepare(&format!(
            "SELECT id FROM review_jobs WHERE {} AND trigger_kind='push' AND status='queued' AND head_sha<>?4",
            same_pr("review_jobs")
        ))?
        .query_map(params![pr.owner, pr.name, pr.number as i64, head_sha], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut cancelled = Vec::new();
    for id in queued {
        // A runner may claim it meanwhile; then it is no longer queued and
        // runs to its end like any other review.
        if finish(db, &id, None, Status::Cancelled, Some(SUPERSEDED_BY_PUSH), now)? {
            cancelled.push(id);
        }
    }
    Ok(cancelled)
}

/// A finding of an earlier review of the same PR, with what became of it.
#[derive(Debug, Clone, PartialEq)]
pub struct PriorFinding {
    pub id: String,
    pub job_id: String,
    /// The head that review looked at.
    pub head_sha: String,
    pub path: String,
    pub line: u32,
    pub start_line: Option<u32>,
    pub side: ReviewSide,
    pub severity: ReviewSeverity,
    pub title: String,
    /// What was said: the person's edit when they edited it.
    pub body: String,
    pub fingerprint: String,
    pub decision: ReviewFindingDecision,
    pub posted: bool,
    pub github_comment_id: Option<String>,
    pub fixed_in_sha: Option<String>,
}

/// Most earlier findings a re-review is shown, the newest kept.
pub const MAX_PRIOR_FINDINGS: usize = 300;

/// Every finding of the PR's earlier reviews that reached a draft, oldest
/// first, leaving out `job_id` itself. Which of them came before the head
/// under review is git's to say: see `prepare`.
pub fn prior_findings(db: &Connection, pr: PrKey<'_>, job_id: &str) -> Result<Vec<PriorFinding>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT * FROM (
            SELECT f.id, f.job_id, j.head_sha, f.path, f.line, f.start_line, f.side, f.severity, f.title,
                COALESCE(f.edited_body, f.body), f.fingerprint, f.decision, f.posted_at IS NOT NULL,
                f.github_comment_id, f.fixed_in_sha, j.created_at AS job_created, f.rowid AS finding_row
            FROM review_findings f JOIN review_jobs j ON j.id=f.job_id
            WHERE {} AND j.id<>?4 AND j.status IN ('drafted','posting','posted')
            ORDER BY j.created_at DESC, f.rowid DESC LIMIT ?5
         ) ORDER BY job_created, finding_row",
        same_pr("j")
    ))?;
    let rows = statement
        .query_map(params![pr.owner, pr.name, pr.number as i64, job_id, MAX_PRIOR_FINDINGS as i64], |row| {
            let side: String = row.get(6)?;
            let severity: String = row.get(7)?;
            let decision: String = row.get(11)?;
            Ok(PriorFinding {
                id: row.get(0)?,
                job_id: row.get(1)?,
                head_sha: row.get(2)?,
                path: row.get(3)?,
                line: row.get::<_, i64>(4)?.max(0) as u32,
                start_line: row.get::<_, Option<i64>>(5)?.map(|line| line.max(0) as u32),
                side: ReviewSide::parse(&side).ok_or_else(|| invalid_column(6, &side))?,
                severity: ReviewSeverity::parse(&severity).ok_or_else(|| invalid_column(7, &severity))?,
                title: row.get(8)?,
                body: row.get(9)?,
                fingerprint: row.get(10)?,
                decision: ReviewFindingDecision::parse(&decision).ok_or_else(|| invalid_column(11, &decision))?,
                posted: row.get(12)?,
                github_comment_id: row.get(13)?,
                fixed_in_sha: row.get(14)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark earlier findings of the same PR fixed at `job_id`'s head. Only one
/// that was kept, edited or posted, and not already marked, is marked: a
/// dropped finding was never a problem to fix. One found at this very head
/// cannot have been fixed by it, nor one of a review queued after this one.
/// That the finding's head is not newer than this one is checked with git
/// when the context is prepared: a pass can only name findings it was shown
/// (`session::known_fixed`).
fn mark_fixed(db: &Connection, job_id: &str, finding_ids: &[String]) -> Result<usize, BridgeError> {
    let mut marked = 0;
    for id in finding_ids {
        marked += db.execute(
            "UPDATE review_findings SET fixed_in_sha=(SELECT head_sha FROM review_jobs WHERE id=?2)
             WHERE id=?1 AND job_id<>?2 AND fixed_in_sha IS NULL
               AND (decision IN ('kept','edited') OR posted_at IS NOT NULL)
               AND job_id IN (SELECT other.id FROM review_jobs other, review_jobs this
                   WHERE this.id=?2 AND other.repo_owner=this.repo_owner COLLATE NOCASE
                     AND other.repo_name=this.repo_name COLLATE NOCASE AND other.pr_number=this.pr_number
                     AND other.head_sha<>this.head_sha AND other.created_at<=this.created_at)",
            params![id, job_id],
        )?;
    }
    Ok(marked)
}

pub fn to_wire(db: &Connection, job: &JobRow) -> Result<wire::ReviewJob, BridgeError> {
    Ok(wire::ReviewJob {
        id: job.id.clone(),
        workspace_id: job.workspace_id.clone(),
        number: job.number,
        head_sha: job.head_sha.clone(),
        status: job.status,
        trigger: job.trigger,
        post_mode: job.post_mode,
        harnesses: slot_harnesses(db, job)?,
        finding_counts: finding_counts(db, job.id.as_str())?,
        incremental_from_sha: job.incremental_from_sha.clone(),
        error: job.error.clone(),
        posted_review_url: job.posted_review_url.clone(),
        retry_of: job.retry_of.clone(),
        created_at: job.created_at.clone(),
        started_at: job.started_at.clone(),
        finished_at: job.finished_at.clone(),
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
