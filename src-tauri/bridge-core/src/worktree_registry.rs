//! The worktree inventory and its retention policy.
//!
//! Bridge cuts a worktree for an isolated chat, for every isolated worker, and
//! for a pull-request checkout. Each of those used to be tracked — if at all —
//! by whichever table happened to need it, so nothing could answer the two
//! questions that keep a laptop alive: *what exists*, and *what is safe to
//! reclaim*. This module owns both.
//!
//! Three rules shape everything here:
//!
//! 1. **Only what Bridge created.** A row is claimed only for a path under the
//!    namespace root Bridge cuts into. Every other checkout `git worktree list`
//!    reports is inventoried as [`STATE_EXTERNAL`] and never touched, because a
//!    developer's own worktree is not Bridge's to collect.
//! 2. **Doubt retains.** Classification must *prove* a checkout is expendable.
//!    Dirty trees, local-only commits, unadopted worker output, and checkouts
//!    git can no longer read are retained with a recorded reason. There is no
//!    force path.
//! 3. **Git runs off the database lock.** Reconcile and sweep read rows under
//!    the mutex, do their filesystem and subprocess work without it, then write
//!    outcomes back — the shape [`crate::worktree_coordinator`] already uses.

use crate::{git, store, BridgeError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// An isolated chat's own checkout.
pub const KIND_ORCHESTRATOR: &str = "orchestrator";
/// A delegated worker's child checkout.
pub const KIND_WORKER: &str = "worker";
/// A pull request checked out for review.
pub const KIND_GITHUB: &str = "github";
/// Somebody else's worktree of a repository Bridge happens to know. Recorded so
/// a storage view can show the whole picture; never Bridge's to reclaim.
pub const KIND_EXTERNAL: &str = "external";

/// Bridge created it and something is using it.
pub const STATE_ACTIVE: &str = "active";
/// Bridge created it and nothing is using it right now.
pub const STATE_IDLE: &str = "idle";
/// Found on disk under the namespace root with no row to explain it.
pub const STATE_ORPHANED: &str = "orphaned";
/// Present on disk but git cannot read it, so nothing about it can be proven.
pub const STATE_UNVERIFIABLE: &str = "unverifiable";
/// A checkout of a repository Bridge knows, outside Bridge's namespace. Recorded
/// for reporting only; never a reclaim candidate.
pub const STATE_EXTERNAL: &str = "external";
/// The directory is gone. The row is kept as history, not as a candidate.
pub const STATE_REMOVED: &str = "removed";

/// The namespace subdirectory each kind is cut into, relative to the worktrees
/// root. Reconcile walks these and only these.
const KIND_DIRECTORIES: &[(&str, &str)] = &[
    ("orchestrators", KIND_ORCHESTRATOR),
    ("workers", KIND_WORKER),
    ("github", KIND_GITHUB),
];

/// How deep a kind's checkouts sit below the worktrees root.
///
/// `orchestrators/<workspace>/<session>` and `workers/<task>/<session>` both
/// nest twice; `github/pr-<n>-<branch>` sits directly under its directory.
/// Walking the wrong depth finds the grouping directory instead of the checkout,
/// which is neither a worktree nor recognisable as one.
fn kind_depth(kind: &str) -> usize {
    match kind {
        KIND_GITHUB => 1,
        _ => 2,
    }
}

/// One inventoried checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRecord {
    pub id: String,
    /// [`KIND_ORCHESTRATOR`], [`KIND_WORKER`], or [`KIND_GITHUB`].
    pub kind: String,
    /// The main checkout this worktree is linked to — where `git worktree
    /// remove` and `git worktree prune` have to run.
    pub repo_root: String,
    pub path: String,
    pub branch: Option<String>,
    pub owner_session_id: Option<String>,
    pub owner_workspace_id: Option<String>,
    /// The commit the checkout was cut from, when the creator recorded one. The
    /// precise base for "did anything land here", independent of remotes.
    pub base_commit: Option<String>,
    pub state: String,
    /// What the last assessment concluded may be done with it — the label of a
    /// [`Disposition`]. `None` until a sweep has looked at it.
    pub disposition: Option<String>,
    pub retained_reason: Option<String>,
    /// When `disposition` was decided. A disposition is a snapshot, never an
    /// authorization: the sweep re-classifies before it removes anything.
    pub assessed_at: Option<String>,
    pub size_bytes: Option<i64>,
    pub size_measured_at: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
}

impl WorktreeRecord {
    fn as_path(&self) -> &Path {
        Path::new(&self.path)
    }

    /// Whether the sweep may consider this row at all. External and already
    /// removed rows are inventory, not candidates.
    ///
    /// A row whose path *is* its repository is never a candidate either. An
    /// in-place worker records the user's own checkout as its worktree, so a
    /// record can name a main working tree — and a main working tree is not
    /// something Bridge cut, however its row got written. Git would refuse to
    /// remove it, but relying on that refusal is luck rather than a guarantee.
    fn is_candidate(&self) -> bool {
        !matches!(self.state.as_str(), STATE_EXTERNAL | STATE_REMOVED)
            && canonical_key(self.as_path()) != canonical_key(Path::new(&self.repo_root))
    }

    fn idle_seconds(&self) -> i64 {
        chrono::DateTime::parse_from_rfc3339(&self.last_used_at)
            .map(|used| {
                Utc::now()
                    .signed_duration_since(used.with_timezone(&Utc))
                    .num_seconds()
            })
            .unwrap_or(0)
    }
}

/// What a caller knows at creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorktree {
    pub kind: String,
    pub repo_root: String,
    pub path: String,
    pub branch: Option<String>,
    pub owner_session_id: Option<String>,
    pub owner_workspace_id: Option<String>,
    pub base_commit: Option<String>,
}

const COLUMNS: &str = "id,kind,repo_root,path,branch,owner_session_id,owner_workspace_id,\
     base_commit,state,disposition,retained_reason,assessed_at,size_bytes,size_measured_at,\
     created_at,last_used_at";

/// The comparison form of a path.
///
/// `git worktree list` reports resolved paths, so on any machine where a
/// worktree sits under a symlink — `/var` and `/tmp` on macOS, a symlinked home
/// or mount anywhere — git's answer and the path Bridge recorded are different
/// strings for the same directory. Comparing them raw makes Bridge file its own
/// worktrees as somebody else's and re-adopt them on every pass.
///
/// Resolving the parent and re-appending the name keeps this usable after the
/// directory is gone, which is exactly when a removal has to find its row.
pub fn canonical_key(path: &Path) -> String {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved.to_string_lossy().into_owned();
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|resolved| resolved.join(name).to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string_lossy().into_owned()),
        _ => path.to_string_lossy().into_owned(),
    }
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorktreeRecord> {
    Ok(WorktreeRecord {
        id: row.get(0)?,
        kind: row.get(1)?,
        repo_root: row.get(2)?,
        path: row.get(3)?,
        branch: row.get(4)?,
        owner_session_id: row.get(5)?,
        owner_workspace_id: row.get(6)?,
        base_commit: row.get(7)?,
        state: row.get(8)?,
        disposition: row.get(9)?,
        retained_reason: row.get(10)?,
        assessed_at: row.get(11)?,
        size_bytes: row.get(12)?,
        size_measured_at: row.get(13)?,
        created_at: row.get(14)?,
        last_used_at: row.get(15)?,
    })
}

/// Record a checkout Bridge just created, or refresh the row for a path it is
/// reusing. Keyed by path: a reused PR checkout must not stack duplicates.
pub fn register(db: &Connection, new: &NewWorktree) -> Result<String, BridgeError> {
    let now = Utc::now().to_rfc3339();
    let path = canonical_key(Path::new(&new.path));
    let repo_root = canonical_key(Path::new(&new.repo_root));
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM worktrees WHERE path=?1",
            params![path],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        db.execute(
            "UPDATE worktrees SET kind=?2,repo_root=?3,branch=?4,
                 owner_session_id=COALESCE(?5,owner_session_id),
                 owner_workspace_id=COALESCE(?6,owner_workspace_id),
                 base_commit=COALESCE(?7,base_commit),
                 state=?8,retained_reason=NULL,last_used_at=?9
             WHERE id=?1",
            params![
                id,
                new.kind,
                repo_root,
                new.branch,
                new.owner_session_id,
                new.owner_workspace_id,
                new.base_commit,
                STATE_ACTIVE,
                now,
            ],
        )?;
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        &format!(
            "INSERT INTO worktrees({COLUMNS}) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,NULL,NULL,NULL,NULL,?10,?11)"
        ),
        params![
            id,
            new.kind,
            repo_root,
            path,
            new.branch,
            new.owner_session_id,
            new.owner_workspace_id,
            new.base_commit,
            STATE_ACTIVE,
            now,
            now,
        ],
    )?;
    Ok(id)
}

/// Record that a directory is gone, whoever removed it.
pub fn mark_removed(db: &Connection, path: &Path, reason: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worktrees SET state=?2,retained_reason=?3 WHERE path=?1",
        params![canonical_key(path), STATE_REMOVED, reason],
    )?;
    Ok(())
}

fn record_assessment(
    db: &Connection,
    id: &str,
    disposition: &Disposition,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worktrees SET disposition=?2,retained_reason=?3,assessed_at=?4 WHERE id=?1",
        params![
            id,
            disposition.label(),
            disposition.reason(),
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

fn set_retained_reason(db: &Connection, id: &str, reason: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worktrees SET retained_reason=?2 WHERE id=?1",
        params![id, reason],
    )?;
    Ok(())
}

fn record_size(db: &Connection, id: &str, bytes: u64) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worktrees SET size_bytes=?2,size_measured_at=?3 WHERE id=?1",
        params![id, bytes as i64, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Bring `last_used_at` up to date from the owning session's own activity.
///
/// A TTL has to measure idleness, not age. Nothing calls a `touch` on every
/// turn — and a hook on the turn path would be one more thing to forget — so
/// the sweep derives the answer instead: the newest recorded event for the
/// session that owns a checkout is the last time anyone used it. Without this,
/// an isolated chat someone works in daily would age out of its own TTL.
pub fn refresh_last_used(db: &Connection) -> Result<(), BridgeError> {
    // One aggregate pass, not a correlated subquery per row (twice per row, at
    // that). `events` carries no index on `entity_id` and grows with all local
    // history, so the correlated form scanned the whole ledger once per
    // worktree per sweep — while holding the shared database lock.
    db.execute(
        "WITH latest AS (
             SELECT e.entity_id AS session_id, MAX(e.created_at) AS used
               FROM events e
              WHERE e.entity_id IN (SELECT owner_session_id FROM worktrees
                                     WHERE owner_session_id IS NOT NULL)
              GROUP BY e.entity_id
         )
         UPDATE worktrees
            SET last_used_at=(SELECT used FROM latest
                               WHERE latest.session_id=worktrees.owner_session_id)
          WHERE owner_session_id IS NOT NULL
            AND EXISTS(SELECT 1 FROM latest
                        WHERE latest.session_id=worktrees.owner_session_id
                          AND latest.used>worktrees.last_used_at)",
        [],
    )?;
    Ok(())
}

/// Flip a checkout nothing is running in from `active` to `idle`. State is
/// bookkeeping for reporting; only [`classify`] decides what may be removed.
fn settle_idle_states(db: &Connection) -> Result<(), BridgeError> {
    db.execute(
        &format!(
            "UPDATE worktrees SET state=?1
              WHERE state=?2
                AND NOT EXISTS(
                    SELECT 1 FROM worker_runtime r LEFT JOIN sessions s ON s.id=r.session_id
                     WHERE r.worktree_path=worktrees.path
                       AND (COALESCE(s.status,'') IN ({LIVE_STATES})
                            OR COALESCE(r.lifecycle_state,'') IN ({LIVE_STATES})))
                AND NOT EXISTS(
                    SELECT 1 FROM sessions
                     WHERE cwd=worktrees.path AND status IN ({LIVE_STATES}))"
        ),
        params![STATE_IDLE, STATE_ACTIVE],
    )?;
    Ok(())
}

/// Every inventoried checkout, newest first.
pub fn records(db: &Connection) -> Result<Vec<WorktreeRecord>, BridgeError> {
    let mut statement =
        db.prepare(&format!("SELECT {COLUMNS} FROM worktrees ORDER BY created_at DESC, path"))?;
    let rows = statement.query_map([], map_record)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(BridgeError::from)
}

/// Candidate rows only — what a sweep may look at.
fn candidates(db: &Connection) -> Result<Vec<WorktreeRecord>, BridgeError> {
    Ok(records(db)?
        .into_iter()
        .filter(WorktreeRecord::is_candidate)
        .collect())
}

// --- retention policy ---------------------------------------------------------

/// What to do with a checkout that is clean and unmerged but whose commits exist
/// on a remote. The work is recoverable, so deleting it loses nothing but a
/// local convenience — which is still the user's call, not Bridge's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushedUnmergedPolicy {
    Retain,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorktreeRetention {
    /// Ceiling for all inventoried checkouts of one repository. A breach the
    /// sweep cannot satisfy safely is reported, never forced.
    pub max_total_bytes: u64,
    /// Ceiling on the number of checkouts per repository.
    pub max_per_repo: usize,
    /// How long a settled worker's checkout may sit unused. Short: its output is
    /// either adopted into the task worktree or deliberately discarded.
    pub worker_idle_ttl: Duration,
    /// How long an isolated chat's checkout may sit unused. Long: a chat is a
    /// place a person comes back to.
    pub orchestrator_idle_ttl: Duration,
    /// How long a reviewed pull request's checkout may sit unused.
    pub github_idle_ttl: Duration,
    pub pushed_unmerged: PushedUnmergedPolicy,
}

impl Default for WorktreeRetention {
    fn default() -> Self {
        Self {
            // A checkout with a populated build cache runs to gigabytes, and a
            // developer machine holds several repositories. Ten gigabytes per
            // repository is enough for a working set and small enough that the
            // sweep engages long before a disk does.
            max_total_bytes: 10 * 1024 * 1024 * 1024,
            max_per_repo: 12,
            worker_idle_ttl: Duration::from_secs(24 * 60 * 60),
            orchestrator_idle_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            github_idle_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            // Retain by default: "it is on the remote" is a good reason to be
            // willing to delete, not a mandate to.
            pushed_unmerged: PushedUnmergedPolicy::Retain,
        }
    }
}

impl WorktreeRetention {
    fn idle_ttl(&self, kind: &str) -> Duration {
        match kind {
            KIND_WORKER => self.worker_idle_ttl,
            KIND_GITHUB => self.github_idle_ttl,
            _ => self.orchestrator_idle_ttl,
        }
    }

    fn is_past_ttl(&self, record: &WorktreeRecord) -> bool {
        record.idle_seconds() >= self.idle_ttl(&record.kind).as_secs() as i64
    }
}

/// What the sweep is allowed to do with one checkout, decided fresh at deletion
/// time rather than carried from a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Clean, and nothing landed here that is not already in the base.
    Reclaimable,
    /// Clean, holds commits, and every one of them exists on a remote.
    PushedUnmerged,
    /// Holds work that exists nowhere else. Never removed.
    AtRisk(String),
    /// Something is using it, or someone still has to decide about it.
    Retained(String),
    /// Nothing about it can be proven, so nothing may be done to it.
    Unverifiable(String),
}

impl Disposition {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Disposition::AtRisk(reason)
            | Disposition::Retained(reason)
            | Disposition::Unverifiable(reason) => Some(reason),
            _ => None,
        }
    }

    /// The label stored on the row and shown to a person.
    pub fn label(&self) -> &'static str {
        match self {
            Disposition::Reclaimable => "reclaimable",
            Disposition::PushedUnmerged => "pushed_unmerged",
            Disposition::AtRisk(_) => "at_risk",
            Disposition::Retained(_) => "retained",
            Disposition::Unverifiable(_) => "unverifiable",
        }
    }
}

/// Lifecycle and session states that mean "in use". A checkout under any of
/// these must survive whatever the caps say.
const LIVE_STATES: &str = "'starting','working','resuming','checkpointing','waiting','warm','restored'";

/// Whether any live session or worker is bound to this path. Covers the
/// borrowed-verifier case: a verifier runs in the implementation worker's
/// checkout under its own `worker_runtime` row.
///
/// Status alone is not enough. A chat whose provider is up but has no turn in
/// flight sits at `ready` — `start_chat` sets exactly that, and says why — so a
/// status-only test would call a live adapter's checkout idle and delete the
/// directory out from under it. `adapter_pid` is the durable claim that a real
/// process owns the session, and boot recovery clears it for every process that
/// is actually gone, so a stale `ready` row from a crashed run does not pin a
/// checkout forever.
fn live_users(db: &Connection, path: &str) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT session_id FROM (
             SELECT r.session_id AS session_id FROM worker_runtime r
               LEFT JOIN sessions s ON s.id=r.session_id
              WHERE r.worktree_path=?1
                AND (COALESCE(s.status,'') IN ({LIVE_STATES})
                     OR COALESCE(r.lifecycle_state,'') IN ({LIVE_STATES})
                     OR s.adapter_pid IS NOT NULL)
             UNION
             SELECT id AS session_id FROM sessions
              WHERE cwd=?1 AND (status IN ({LIVE_STATES}) OR adapter_pid IS NOT NULL)
         ) ORDER BY session_id"
    ))?;
    let rows = statement.query_map(params![path], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(BridgeError::from)
}

/// Whether a worker's output in this checkout is still awaiting the user's
/// adopt-or-discard decision.
fn unadopted_binding(db: &Connection, path: &str) -> Result<bool, BridgeError> {
    Ok(db
        .query_row(
            "SELECT 1 FROM worker_worktree_adoptions
              WHERE worktree_path=?1 AND state='pending_adoption' LIMIT 1",
            params![path],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false))
}

/// Everything classification needs from the database, read once under the lock
/// so the git work that follows holds nothing.
#[derive(Debug, Clone)]
struct ClassificationFacts {
    live_users: Vec<String>,
    unadopted: bool,
}

fn classification_facts(
    db: &Connection,
    record: &WorktreeRecord,
) -> Result<ClassificationFacts, BridgeError> {
    Ok(ClassificationFacts {
        live_users: live_users(db, &record.path)?,
        unadopted: unadopted_binding(db, &record.path)?,
    })
}

/// Decide what may be done with one checkout. Pure apart from git reads; takes
/// its database facts as a value so it can run without the lock.
fn classify_with(record: &WorktreeRecord, facts: &ClassificationFacts) -> Disposition {
    if record.state == STATE_EXTERNAL {
        return Disposition::Retained("outside Bridge's worktree namespace".into());
    }
    let path = record.as_path();
    if !path.is_dir() {
        return Disposition::Reclaimable;
    }
    if !facts.live_users.is_empty() {
        return Disposition::Retained(format!(
            "{} is still running in it",
            facts.live_users.join(", ")
        ));
    }

    // Ask git before trusting any local reasoning: a checkout whose
    // administrative directory is gone answers every question with an error,
    // and an error must not read as "clean".
    let divergence = git::base_branch_divergence(path, false);
    let Some(head) = divergence.head.clone().or_else(|| git::head_commit(path)) else {
        return Disposition::Unverifiable(
            "git cannot read this checkout, so its contents cannot be verified".into(),
        );
    };
    if divergence.dirty {
        return Disposition::AtRisk("uncommitted changes".into());
    }

    // How much landed here. The recorded base commit is exact and remote-free;
    // fall back to the default-branch comparison, and refuse to guess when
    // neither is available.
    let ahead = match record.base_commit.as_deref() {
        Some(base) => match git::commits_ahead_of(path, base) {
            Ok(count) => count,
            Err(_) => {
                return Disposition::AtRisk(
                    "the commit this checkout was cut from is no longer readable".into(),
                )
            }
        },
        None => {
            if divergence.base_ref.is_none() {
                return Disposition::AtRisk(
                    "no base commit or branch is available to compare against".into(),
                );
            }
            divergence.ahead
        }
    };

    if facts.unadopted && ahead > 0 {
        return Disposition::Retained(
            "a worker's output here has not been adopted or discarded yet".into(),
        );
    }
    if ahead == 0 {
        return Disposition::Reclaimable;
    }
    match git::remote_refs_contain(path, &head) {
        Ok(true) => Disposition::PushedUnmerged,
        Ok(false) => Disposition::AtRisk(format!(
            "{ahead} commit(s) exist only in this checkout"
        )),
        // An unanswerable question is not a yes.
        Err(_) => Disposition::AtRisk(format!(
            "{ahead} commit(s) here could not be found on any remote"
        )),
    }
}

/// Classify one recorded checkout. Convenience for callers holding a single
/// connection; the sweep uses the phased pair.
pub fn classify(db: &Connection, record: &WorktreeRecord) -> Result<Disposition, BridgeError> {
    let facts = classification_facts(db, record)?;
    Ok(classify_with(record, &facts))
}

// --- reporting ----------------------------------------------------------------

/// One inventoried checkout, as a client sees it.
///
/// `disposition` is the last assessment, not a live answer: deciding it costs
/// several git invocations per checkout, so a read of the inventory reports what
/// the last sweep concluded and `assessedAt` says how fresh that is. Nothing
/// acts on this value — the sweep re-classifies at the moment of deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInventoryEntry {
    pub id: String,
    pub kind: String,
    pub repo_root: String,
    pub path: String,
    pub branch: Option<String>,
    pub owner_session_id: Option<String>,
    pub owner_workspace_id: Option<String>,
    pub state: String,
    pub disposition: Option<String>,
    pub retained_reason: Option<String>,
    pub assessed_at: Option<String>,
    pub size_bytes: Option<i64>,
    pub size_measured_at: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
    pub idle_seconds: i64,
}

/// Per-repository rollup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRepositoryUsage {
    pub repo_root: String,
    pub count: i64,
    pub size_bytes: i64,
    /// Bytes the sweep could reclaim without a human decision.
    pub reclaimable_bytes: i64,
    pub over_budget: bool,
}

/// What the worktrees cost and what the caps are — the numbers a storage
/// surface renders, and the ones that make a refusal explicable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUsage {
    pub total_count: i64,
    pub total_bytes: i64,
    pub reclaimable_count: i64,
    pub reclaimable_bytes: i64,
    /// Checkouts nothing may remove without a person deciding.
    pub retained_count: i64,
    pub max_total_bytes: i64,
    pub max_per_repo: i64,
    pub worker_idle_ttl_seconds: i64,
    pub orchestrator_idle_ttl_seconds: i64,
    pub github_idle_ttl_seconds: i64,
    pub repositories: Vec<WorktreeRepositoryUsage>,
}

fn entry(record: WorktreeRecord) -> WorktreeInventoryEntry {
    let idle_seconds = record.idle_seconds();
    WorktreeInventoryEntry {
        id: record.id,
        kind: record.kind,
        repo_root: record.repo_root,
        path: record.path,
        branch: record.branch,
        owner_session_id: record.owner_session_id,
        owner_workspace_id: record.owner_workspace_id,
        state: record.state,
        disposition: record.disposition,
        retained_reason: record.retained_reason,
        assessed_at: record.assessed_at,
        size_bytes: record.size_bytes,
        size_measured_at: record.size_measured_at,
        created_at: record.created_at,
        last_used_at: record.last_used_at,
        idle_seconds,
    }
}

/// The inventory, excluding rows kept only as history.
pub fn inventory(db: &Connection) -> Result<Vec<WorktreeInventoryEntry>, BridgeError> {
    Ok(records(db)?
        .into_iter()
        .filter(|record| record.state != STATE_REMOVED)
        .map(entry)
        .collect())
}

pub fn usage(
    db: &Connection,
    retention: &WorktreeRetention,
) -> Result<WorktreeUsage, BridgeError> {
    let rows = records(db)?
        .into_iter()
        .filter(|record| record.state != STATE_REMOVED)
        .collect::<Vec<_>>();
    let reclaimable = |record: &WorktreeRecord| {
        matches!(record.disposition.as_deref(), Some("reclaimable"))
            || (retention.pushed_unmerged == PushedUnmergedPolicy::Delete
                && matches!(record.disposition.as_deref(), Some("pushed_unmerged")))
    };
    let mut repositories: HashMap<String, WorktreeRepositoryUsage> = HashMap::new();
    let mut total = WorktreeUsage {
        total_count: 0,
        total_bytes: 0,
        reclaimable_count: 0,
        reclaimable_bytes: 0,
        retained_count: 0,
        max_total_bytes: retention.max_total_bytes as i64,
        max_per_repo: retention.max_per_repo as i64,
        worker_idle_ttl_seconds: retention.worker_idle_ttl.as_secs() as i64,
        orchestrator_idle_ttl_seconds: retention.orchestrator_idle_ttl.as_secs() as i64,
        github_idle_ttl_seconds: retention.github_idle_ttl.as_secs() as i64,
        repositories: Vec::new(),
    };
    for record in rows {
        let bytes = record.size_bytes.unwrap_or(0).max(0);
        let counts_against_cap = record.state != STATE_EXTERNAL;
        total.total_count += 1;
        total.total_bytes += bytes;
        if reclaimable(&record) {
            total.reclaimable_count += 1;
            total.reclaimable_bytes += bytes;
        } else if counts_against_cap {
            total.retained_count += 1;
        }
        let rollup = repositories
            .entry(record.repo_root.clone())
            .or_insert_with(|| WorktreeRepositoryUsage {
                repo_root: record.repo_root.clone(),
                count: 0,
                size_bytes: 0,
                reclaimable_bytes: 0,
                over_budget: false,
            });
        if counts_against_cap {
            rollup.count += 1;
            rollup.size_bytes += bytes;
            if reclaimable(&record) {
                rollup.reclaimable_bytes += bytes;
            }
        }
    }
    let mut repositories = repositories.into_values().collect::<Vec<_>>();
    for rollup in &mut repositories {
        rollup.over_budget = rollup.size_bytes as u64 >= retention.max_total_bytes
            || rollup.count as usize >= retention.max_per_repo;
    }
    repositories.sort_by(|left, right| {
        right
            .size_bytes
            .cmp(&left.size_bytes)
            .then_with(|| left.repo_root.cmp(&right.repo_root))
    });
    total.repositories = repositories;
    Ok(total)
}

/// Recreate a checkout the sweep reclaimed, from the branch the inventory
/// recorded. Returns whether a worktree was restored.
///
/// Reclaiming a checkout must not cost the session that owns it. `start_chat`
/// takes `sessions.cwd` verbatim and only `create_dir_all`s it, so a resumed
/// chat whose worktree had been collected would start in an empty directory
/// that is not a repository at all — its project silently gone. Branch refs are
/// never deleted by the sweep, so the branch is always still there to check out
/// again; this is the same restore [`crate::worktree_coordinator`] already does
/// for a pull-request checkout whose tree was reclaimed.
///
/// Failure is not an error: the caller falls back to its previous behaviour.
pub fn restore_if_reclaimed(db: &Mutex<Connection>, path: &Path) -> bool {
    if path.is_dir() {
        return false;
    }
    let key = canonical_key(path);
    let Some(record) = ({
        let db = db.lock().unwrap();
        records(&db)
            .ok()
            .and_then(|rows| rows.into_iter().find(|row| row.path == key))
    }) else {
        return false;
    };
    // Never restore something Bridge did not cut, and never guess a branch.
    if record.state == STATE_EXTERNAL || record.kind == KIND_EXTERNAL {
        return false;
    }
    let (Some(branch), repo) = (record.branch.as_deref(), PathBuf::from(&record.repo_root)) else {
        return false;
    };
    if branch.is_empty() || !repo.is_dir() {
        return false;
    }
    if git::create_worktree_on_branch(&repo, path, branch, "origin").is_err() {
        return false;
    }
    let db = db.lock().unwrap();
    let _ = register(
        &db,
        &NewWorktree {
            kind: record.kind.clone(),
            repo_root: record.repo_root.clone(),
            path: record.path.clone(),
            branch: Some(branch.to_owned()),
            owner_session_id: record.owner_session_id.clone(),
            owner_workspace_id: record.owner_workspace_id.clone(),
            base_commit: None,
        },
    );
    let _ = store::event(
        &db,
        "worktree",
        "worktree.restored",
        &record.path,
        &format!("Restored reclaimed {} worktree on branch {branch}", record.kind),
    );
    true
}

// --- reconcile ----------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Registrations git was still holding for directories that are gone.
    pub pruned_registrations: usize,
    /// Rows whose directory has disappeared.
    pub marked_removed: usize,
    /// Directories under the namespace root that had no row.
    pub adopted_orphans: usize,
    /// Adopted directories git could not read.
    pub unverifiable: usize,
    /// Checkouts of a known repository outside Bridge's namespace.
    pub external: usize,
}

impl ReconcileOutcome {
    pub fn is_quiet(&self) -> bool {
        self.pruned_registrations == 0
            && self.marked_removed == 0
            && self.adopted_orphans == 0
            && self.unverifiable == 0
    }
}

/// Bring the inventory, git's registrations, and the filesystem back into
/// agreement. Runs at boot and on the maintenance tick.
///
/// Drift is not an error state: a crash between creating a worktree and writing
/// its row, a user deleting a directory by hand, or an out-of-band
/// `git worktree prune` are all ordinary. What matters is that afterwards
/// nothing is invisible.
pub fn reconcile(
    db: &Mutex<Connection>,
    namespace_root: &Path,
) -> Result<ReconcileOutcome, BridgeError> {
    let mut outcome = ReconcileOutcome::default();
    let known = { candidates(&db.lock().unwrap())? };

    // Every repository worth asking git about. Seeded from the repositories
    // Bridge already knows — its projects and workspaces — not only from rows
    // that happen to exist, or a fresh install with no Bridge worktrees yet
    // would never notice a stale registration or report a developer's own
    // checkout. Grows further as orphan adoption finds repositories nothing
    // else names.
    let mut repos: Vec<PathBuf> = {
        let db = db.lock().unwrap();
        let mut statement = db.prepare(
            "SELECT path FROM projects WHERE COALESCE(path,'')<>''
             UNION
             SELECT path FROM workspaces WHERE COALESCE(path,'')<>''",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(Result::ok).map(PathBuf::from).collect()
    };

    // Rows whose directory is gone: record that, and let git drop the
    // registration it may still be holding.
    for record in &known {
        let repo = PathBuf::from(&record.repo_root);
        if !repos.contains(&repo) {
            repos.push(repo);
        }
        if record.as_path().is_dir() {
            continue;
        }
        db.lock()
            .unwrap()
            .execute(
                "UPDATE worktrees SET state=?2,retained_reason=?3 WHERE id=?1",
                params![record.id, STATE_REMOVED, "the directory is gone"],
            )
            .map(|_| ())?;
        outcome.marked_removed += 1;
    }

    // Directories under Bridge's namespace with no row. Adopt them: an
    // unrecorded checkout is worse than a stale one, because nothing can report
    // it, let alone reclaim it.
    // Keyed canonically so a row written before this normalization existed — a
    // backfilled path, say — still matches the directory it names.
    let mut recorded: HashMap<String, ()> = known
        .iter()
        .map(|record| (canonical_key(record.as_path()), ()))
        .collect();
    for (directory, kind) in KIND_DIRECTORIES {
        let root = namespace_root.join(directory);
        for path in directories_at_depth(&root, kind_depth(kind)) {
            let path_text = canonical_key(&path);
            if recorded.contains_key(&path_text) {
                continue;
            }
            let readable = git::main_worktree_root(&path);
            // A checkout git refuses to read still names its repository in its
            // own `.git` pointer file. Falling back to it keeps an unverifiable
            // checkout attributable — and counted against the right budget —
            // instead of belonging to nothing.
            let repo_root = readable
                .clone()
                .or_else(|| git::linked_worktree_repo_root(&path));
            let readable = readable.is_some();
            let adopted_repo_root = repo_root
                .as_ref()
                .map(|root| root.to_string_lossy().to_string());
            let new = NewWorktree {
                kind: (*kind).to_owned(),
                repo_root: repo_root
                    .map(|root| root.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path_text.clone(),
                branch: git::current_branch(&path),
                owner_session_id: None,
                owner_workspace_id: None,
                base_commit: None,
            };
            let (state, reason) = if readable {
                (STATE_ORPHANED, "found on disk with no Bridge record")
            } else {
                (
                    STATE_UNVERIFIABLE,
                    "found on disk, but git cannot read it; nothing about it can be verified",
                )
            };
            {
                let db = db.lock().unwrap();
                let id = register(&db, &new)?;
                db.execute(
                    "UPDATE worktrees SET state=?2,retained_reason=?3 WHERE id=?1",
                    params![id, state, reason],
                )?;
            }
            if let Some(root) = adopted_repo_root.filter(|_| readable) {
                let root = PathBuf::from(root);
                if root.is_dir() && !repos.contains(&root) {
                    // Scan it in this pass rather than the next: an adopted
                    // orphan is often the only thing naming its repository.
                    repos.push(root);
                }
            }
            recorded.insert(path_text, ());
            outcome.adopted_orphans += 1;
            if !readable {
                outcome.unverifiable += 1;
            }
        }
    }

    // Ask git, per repository, what it still believes.
    for repo in repos {
        if !repo.is_dir() {
            continue;
        }
        let Ok(entries) = git::list_worktrees(&repo) else {
            continue;
        };
        let mut prunable = 0usize;
        let canonical_repo = canonical_key(&repo);
        let canonical_namespace = canonical_key(namespace_root);
        for entry in entries {
            if entry.bare || canonical_key(&entry.path) == canonical_repo {
                continue;
            }
            if entry.prunable || !entry.path.is_dir() {
                prunable += 1;
                continue;
            }
            let path_text = canonical_key(&entry.path);
            if recorded.contains_key(&path_text) {
                continue;
            }
            if path_text.starts_with(&canonical_namespace) {
                // Under Bridge's namespace but not at a layout depth reconcile
                // walks. Record it so it is at least visible.
                continue;
            }
            // Somebody else's worktree of a repository Bridge happens to know.
            // Inventory it; never claim it.
            let db = db.lock().unwrap();
            let id = register(
                &db,
                &NewWorktree {
                    kind: KIND_EXTERNAL.to_owned(),
                    repo_root: repo.to_string_lossy().to_string(),
                    path: path_text.clone(),
                    branch: entry.branch.clone(),
                    owner_session_id: None,
                    owner_workspace_id: None,
                    base_commit: None,
                },
            )?;
            db.execute(
                "UPDATE worktrees SET state=?2,retained_reason=?3 WHERE id=?1",
                params![
                    id,
                    STATE_EXTERNAL,
                    "outside Bridge's worktree namespace; reported but never reclaimed"
                ],
            )?;
            drop(db);
            recorded.insert(path_text, ());
            outcome.external += 1;
        }
        if prunable > 0 && git::prune_worktrees(&repo).is_ok() {
            outcome.pruned_registrations += prunable;
        }
    }

    Ok(outcome)
}

/// Immediate subdirectories exactly `depth` levels below `root`. Bounded by the
/// layout rather than recursive: a checkout's own contents are never candidates.
///
/// Symlinks are skipped rather than followed. `is_dir` follows them, and a
/// symlink dropped into a layout slot — which anything with write access to its
/// own checkout can create as a sibling — would otherwise be inventoried as a
/// Bridge checkout at its *resolved* path, outside the namespace entirely. That
/// is how a sweeper ends up deleting a developer's own worktree.
fn directories_at_depth(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for directory in level {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(metadata) = entry.path().symlink_metadata() else {
                    continue;
                };
                if metadata.is_dir() {
                    next.push(entry.path());
                }
            }
        }
        level = next;
    }
    level
}

/// Whether a path lies inside the namespace Bridge cuts its own worktrees into.
///
/// The invariant this whole module rests on is "only what Bridge created", and
/// the namespace root is what makes that checkable. Every reclaim decision
/// re-asks the question at the moment of deletion rather than trusting that
/// whatever wrote the row got it right.
fn inside_namespace(path: &Path, namespace_root: &Path) -> bool {
    Path::new(&canonical_key(path)).starts_with(canonical_key(namespace_root))
}

// --- size ---------------------------------------------------------------------

/// Upper bound on entries one measurement will visit. A populated Rust target
/// directory runs to a few hundred thousand files; beyond this the answer is
/// "large" and walking further buys nothing.
const SIZE_WALK_ENTRY_LIMIT: usize = 400_000;

/// Bytes on disk under a path, and whether the walk saw all of it.
///
/// Best effort by design: an unreadable subdirectory lowers the estimate rather
/// than failing the sweep, and hitting the entry bound returns what was counted
/// so far. Both bias the number *downward*, which biases a cap decision toward
/// keeping a checkout — the safe direction.
pub fn directory_size(path: &Path) -> (u64, bool) {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > SIZE_WALK_ENTRY_LIMIT {
                return (total, false);
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                total += metadata.len();
            }
        }
    }
    (total, true)
}

// --- capacity -----------------------------------------------------------------

/// Bytes and count currently inventoried for one repository. Reads recorded
/// sizes only — cheap enough for the creation path, where a filesystem walk
/// would put a directory scan in front of every delegation.
pub fn repo_usage(db: &Connection, repo_root: &str) -> Result<(u64, usize), BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {COLUMNS} FROM worktrees WHERE repo_root=?1 AND state NOT IN (?2,?3)"
    ))?;
    // Canonical, like every path this module stores: the workspace path a
    // caller hands us comes straight from its own row and may name the same
    // directory by a different route.
    let rows = statement.query_map(
        params![
            canonical_key(Path::new(repo_root)),
            STATE_REMOVED,
            STATE_EXTERNAL
        ],
        map_record,
    )?;
    let mut bytes = 0u64;
    let mut count = 0usize;
    for record in rows {
        let record = record?;
        bytes += record.size_bytes.unwrap_or(0).max(0) as u64;
        count += 1;
    }
    Ok((bytes, count))
}

/// Why a new checkout cannot be cut right now, if it cannot. Consulted by the
/// creation gate; a `Some` here is a queue-or-refuse signal, never a deletion.
pub fn over_capacity(
    db: &Connection,
    repo_root: &str,
    retention: &WorktreeRetention,
) -> Result<Option<String>, BridgeError> {
    let (bytes, count) = repo_usage(db, repo_root)?;
    if count >= retention.max_per_repo {
        return Ok(Some(format!(
            "this repository already has {count} Bridge worktrees (limit {}); reclaim one first",
            retention.max_per_repo
        )));
    }
    if bytes >= retention.max_total_bytes {
        return Ok(Some(format!(
            "Bridge worktrees for this repository use {} (limit {}); reclaim one first",
            human_bytes(bytes),
            human_bytes(retention.max_total_bytes)
        )));
    }
    Ok(None)
}

/// Whether a child worktree may be cut for this repository. Replaces a check
/// that only asked whether the namespace root had a parent directory.
pub fn has_capacity(
    db: &Connection,
    repo_root: &str,
    retention: &WorktreeRetention,
) -> Result<bool, BridgeError> {
    Ok(over_capacity(db, repo_root, retention)?.is_none())
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// --- sweep --------------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepOutcome {
    pub removed: usize,
    pub removed_bytes: u64,
    pub retained: usize,
    pub retained_bytes: u64,
    /// How far a repository's retained set still sits above its byte cap after
    /// the sweep did everything it could safely do. Non-zero means the breach
    /// can only be cleared by a human decision.
    pub over_budget_bytes: u64,
    /// Removals that were decided but failed. One stuck directory must not
    /// stall the rest of the pass.
    pub skipped: usize,
    /// Size measurements that hit their entry bound. Those sizes are
    /// underestimates, so a cap decision made from them is conservative — it
    /// under-sweeps rather than over-sweeps — but a byte budget quietly
    /// computed from partial numbers is worth saying out loud.
    pub measurements_truncated: usize,
}

impl SweepOutcome {
    pub fn is_quiet(&self) -> bool {
        self.removed == 0
            && self.skipped == 0
            && self.over_budget_bytes == 0
            && self.measurements_truncated == 0
    }
}

/// One candidate, classified and measured.
struct Assessed {
    record: WorktreeRecord,
    disposition: Disposition,
    bytes: u64,
    /// Whether a measurement actually ran. A missing directory is not zero
    /// bytes; it is unknown, and must not overwrite a recorded size.
    measured: bool,
}

/// Whether a disposition authorizes removal under this policy. The single place
/// that answers it, so the plan and the re-check at deletion time cannot drift.
fn is_removable(disposition: &Disposition, retention: &WorktreeRetention) -> bool {
    match disposition {
        Disposition::Reclaimable => true,
        Disposition::PushedUnmerged => {
            retention.pushed_unmerged == PushedUnmergedPolicy::Delete
        }
        _ => false,
    }
}

/// Reclaim what can be proven expendable, under the retention policy.
///
/// Phased so no git or filesystem work happens under the database lock: rows and
/// their database facts are read first, classification and measurement run
/// unlocked, and only the outcome is written back.
pub fn sweep(
    db: &Mutex<Connection>,
    namespace_root: &Path,
    retention: &WorktreeRetention,
) -> Result<SweepOutcome, BridgeError> {
    let mut outcome = SweepOutcome::default();

    let assessable = {
        let db = db.lock().unwrap();
        refresh_last_used(&db)?;
        settle_idle_states(&db)?;
        let mut facts = Vec::new();
        for record in candidates(&db)? {
            // Containment is checked here *and* again immediately before
            // deletion: a row can name a path outside the namespace however it
            // was written, and such a path is not Bridge's to remove.
            if !inside_namespace(record.as_path(), namespace_root) {
                continue;
            }
            let record_facts = classification_facts(&db, &record)?;
            facts.push((record, record_facts));
        }
        facts
    };

    let mut assessed: Vec<Assessed> = Vec::new();
    for (record, facts) in assessable {
        let disposition = classify_with(&record, &facts);
        let measured = record.as_path().is_dir();
        let (bytes, complete) = if measured {
            directory_size(record.as_path())
        } else {
            (0, true)
        };
        if !complete {
            outcome.measurements_truncated += 1;
        }
        assessed.push(Assessed {
            record,
            disposition,
            bytes,
            measured,
        });
    }

    {
        let db = db.lock().unwrap();
        for item in &assessed {
            // Zero is a real answer. Skipping it left the old, larger size in
            // place after a checkout's build output was deleted, so the byte cap
            // went on refusing new workers for space that had been freed.
            if item.measured {
                record_size(&db, &item.record.id, item.bytes)?;
            }
            record_assessment(&db, &item.record.id, &item.disposition)?;
        }
    }

    // Decide per repository: the caps are per repository, and a busy repo must
    // not spend another's budget.
    let mut by_repo: HashMap<String, Vec<Assessed>> = HashMap::new();
    for item in assessed {
        by_repo
            .entry(item.record.repo_root.clone())
            .or_default()
            .push(item);
    }

    for (_repo, mut items) in by_repo {
        // Oldest-idle first: when a cap forces a choice, the least recently
        // useful checkout goes first.
        items.sort_by_key(|item| std::cmp::Reverse(item.record.idle_seconds()));

        let removable = |item: &Assessed| is_removable(&item.disposition, retention);

        // The running plan's view of what would remain. Only used to decide
        // what to attempt; the outcome is measured afterwards.
        let mut live_bytes: u64 = items.iter().map(|item| item.bytes).sum();
        let mut live_count = items.len();
        let mut planned: Vec<usize> = Vec::new();
        // Past its TTL and expendable: collect it.
        for (index, item) in items.iter().enumerate() {
            if removable(item) && retention.is_past_ttl(&item.record) {
                planned.push(index);
                live_bytes = live_bytes.saturating_sub(item.bytes);
                live_count -= 1;
            }
        }
        // Still over a cap: keep taking expendable checkouts, oldest first,
        // even inside their TTL. A cap is a promise about the machine.
        for (index, item) in items.iter().enumerate() {
            // Strictly below the caps, not merely at them. The creation gate
            // refuses at `>= max_per_repo`, so stopping at equality left the
            // repository permanently full: the tick reclaimed nothing and every
            // queued delegation waited for a TTL that had no reason to expire.
            // Clearing a cap has to leave room for the work it is blocking.
            if live_bytes < retention.max_total_bytes && live_count < retention.max_per_repo {
                break;
            }
            if planned.contains(&index) || !removable(item) {
                continue;
            }
            planned.push(index);
            live_bytes = live_bytes.saturating_sub(item.bytes);
            live_count -= 1;
        }

        // Account for what actually happened, not what was planned. A removal
        // that git refused leaves the checkout on disk, so it still owes its
        // bytes to the budget — planning alone must not be able to hide a
        // breach.
        let mut removed: Vec<usize> = Vec::new();
        for index in &planned {
            let item = &items[*index];
            match remove_recorded_worktree(db, namespace_root, &item.record, item.bytes, retention) {
                Ok(true) => {
                    outcome.removed += 1;
                    outcome.removed_bytes += item.bytes;
                    removed.push(*index);
                }
                Ok(false) | Err(_) => outcome.skipped += 1,
            }
        }

        let mut surviving_bytes = 0u64;
        for (index, item) in items.iter().enumerate() {
            if removed.contains(&index) {
                continue;
            }
            outcome.retained += 1;
            outcome.retained_bytes += item.bytes;
            surviving_bytes += item.bytes;
        }
        // What safety cost the budget. Reported rather than acted on: whatever
        // is left is exactly what nothing may delete.
        outcome.over_budget_bytes += surviving_bytes.saturating_sub(retention.max_total_bytes);
    }

    Ok(outcome)
}

/// Remove one recorded checkout and write the audit trail. Returns whether the
/// directory is actually gone afterwards.
///
/// Every guarantee this module makes is re-established here, at the moment of
/// deletion, because the plan that selected this checkout ran without the
/// database lock and against a filesystem that has since moved on:
///
/// - the path must still be inside Bridge's namespace,
/// - nothing may have started running in it,
/// - and the **full** classification must still authorize removal.
///
/// The last of those is not covered by `safe_remove_worker_worktree`. That only
/// re-runs `git status`, and `git worktree remove` needs `--force` for a dirty
/// or locked tree — not for a clean one carrying a commit that exists nowhere
/// else. A commit made between the scan and this call would have been deleted
/// with the directory.
fn remove_recorded_worktree(
    db: &Mutex<Connection>,
    namespace_root: &Path,
    record: &WorktreeRecord,
    bytes: u64,
    retention: &WorktreeRetention,
) -> Result<bool, BridgeError> {
    let path = record.as_path();
    if !path.is_dir() {
        let db = db.lock().unwrap();
        mark_removed(&db, path, "the directory was already gone")?;
        return Ok(true);
    }
    if !inside_namespace(path, namespace_root) {
        let db = db.lock().unwrap();
        let reason = "outside Bridge's worktree namespace".to_owned();
        set_retained_reason(&db, &record.id, &reason)?;
        let _ = store::event(&db, "worktree", "worktree.retained", &record.path, &reason);
        return Ok(false);
    }

    // Re-decide from scratch, against facts read now.
    let (fresh, facts) = {
        let db = db.lock().unwrap();
        let fresh = records(&db)?
            .into_iter()
            .find(|row| row.id == record.id)
            .unwrap_or_else(|| record.clone());
        let facts = classification_facts(&db, &fresh)?;
        (fresh, facts)
    };
    let disposition = classify_with(&fresh, &facts);
    if !is_removable(&disposition, retention) {
        let reason = disposition
            .reason()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("no longer {}", disposition.label()));
        let db = db.lock().unwrap();
        record_assessment(&db, &record.id, &disposition)?;
        let _ = store::event(&db, "worktree", "worktree.retained", &record.path, &reason);
        return Ok(false);
    }

    let repo = PathBuf::from(&record.repo_root);
    let head = git::head_commit(path).unwrap_or_else(|| "unknown".to_owned());
    if let Err(error) = git::safe_remove_worker_worktree(&repo, path, false) {
        let db = db.lock().unwrap();
        set_retained_reason(&db, &record.id, &error.to_string())?;
        let _ = store::event(
            &db,
            "worktree",
            "worktree.retained",
            &record.path,
            &error.to_string(),
        );
        return Ok(false);
    }
    if path.is_dir() {
        return Ok(false);
    }
    let db = db.lock().unwrap();
    // An unsettled binding that outlives its checkout is worse than a stale
    // directory: `pending_for_parent` keeps offering the user an adopt-or-discard
    // decision about a path that no longer exists, and the worker-side collector
    // skips it precisely because the path is missing. Only an *empty* checkout
    // ever reaches this line, so `empty` is the truthful terminal state.
    settle_empty_binding(&db, &record.path)?;
    mark_removed(&db, path, disposition.label())?;
    let _ = store::event(
        &db,
        "worktree",
        "worktree.reclaimed",
        &record.path,
        &format!(
            "Reclaimed {} worktree {} (branch {}, head {}, {}) — freed {}",
            record.kind,
            record.path,
            record.branch.as_deref().unwrap_or("detached"),
            head,
            disposition.label(),
            human_bytes(bytes),
        ),
    );
    Ok(true)
}

/// Settle any unadopted worker binding on this path as `empty`.
///
/// Matched by canonical path in Rust rather than SQL: a binding records
/// whatever path its creator held, which need not be the normalized form the
/// inventory stores.
fn settle_empty_binding(db: &Connection, canonical_path: &str) -> Result<(), BridgeError> {
    let pending = {
        let mut statement = db.prepare(
            "SELECT session_id,worktree_path FROM worker_worktree_adoptions
              WHERE state='pending_adoption'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (session_id, recorded) in pending {
        if canonical_key(Path::new(&recorded)) != canonical_path {
            continue;
        }
        crate::worker_adoption::settle(
            db,
            &session_id,
            crate::worker_adoption::STATE_EMPTY,
            "the worktree held nothing to adopt and was reclaimed",
        )?;
        // Same treatment settlement would have given it, under the same guard:
        // the checkout is gone and its commits are provably elsewhere, so the
        // scratch ref goes too.
        crate::worker_adoption::release_branch_for(db, &session_id);
    }
    Ok(())
}

/// What an explicit reclaim did, or why it did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeReclaimResult {
    pub reclaimed: bool,
    pub bytes_freed: i64,
    /// The disposition the reclaim decided against, fresh.
    pub disposition: String,
    /// Why it was refused, in words meant for a person. `None` on success.
    pub detail: Option<String>,
}

/// What archiving a chat did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveChatResult {
    pub archived: bool,
    pub bytes_freed: i64,
    /// Why the chat's checkout was kept, when it was. `None` means there was
    /// nothing to keep or it was reclaimed.
    pub worktree_detail: Option<String>,
}

/// Reclaim one inventoried checkout because a person asked for it.
///
/// The classification is the sweep's, and the refusals are the sweep's: at-risk,
/// retained and unverifiable checkouts come back refused with their reason
/// rather than as an error, because "no, and here is why" is the useful answer
/// for a button.
///
/// One deliberate difference. A checkout whose every commit is already on a
/// remote — `pushed_unmerged` — is retained by the unattended sweep and
/// reclaimed here, because the difference is who is asking: a person clicking
/// Reclaim on a row that says so has been told what they are discarding. The
/// sweep has nobody to tell. Nothing unique to this disk is removed on either
/// path.
pub fn reclaim(
    db: &Mutex<Connection>,
    namespace_root: &Path,
    worktree_id: &str,
    retention: &WorktreeRetention,
) -> Result<WorktreeReclaimResult, BridgeError> {
    let record = {
        let db = db.lock().unwrap();
        records(&db)?
            .into_iter()
            .find(|row| row.id == worktree_id)
            .ok_or_else(|| BridgeError::Invalid(format!("no worktree {worktree_id} is recorded")))?
    };
    if !record.is_candidate() {
        return Ok(WorktreeReclaimResult {
            reclaimed: false,
            bytes_freed: 0,
            disposition: STATE_EXTERNAL.to_owned(),
            detail: Some(
                "this checkout is not one Bridge created, so Bridge will not remove it".into(),
            ),
        });
    }
    if !inside_namespace(record.as_path(), namespace_root) {
        return Ok(WorktreeReclaimResult {
            reclaimed: false,
            bytes_freed: 0,
            disposition: STATE_EXTERNAL.to_owned(),
            detail: Some("this checkout sits outside Bridge's worktree namespace".into()),
        });
    }

    let explicit = WorktreeRetention {
        pushed_unmerged: PushedUnmergedPolicy::Delete,
        ..*retention
    };
    let facts = { classification_facts(&db.lock().unwrap(), &record)? };
    let disposition = classify_with(&record, &facts);
    if !is_removable(&disposition, &explicit) {
        let detail = disposition
            .reason()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("this checkout is {}", disposition.label()));
        {
            let db = db.lock().unwrap();
            record_assessment(&db, &record.id, &disposition)?;
        }
        return Ok(WorktreeReclaimResult {
            reclaimed: false,
            bytes_freed: 0,
            disposition: disposition.label().to_owned(),
            detail: Some(detail),
        });
    }

    let bytes = if record.as_path().is_dir() {
        directory_size(record.as_path()).0
    } else {
        0
    };
    // `remove_recorded_worktree` re-decides for itself, so a race between the
    // classification above and this call still cannot destroy work.
    let reclaimed = remove_recorded_worktree(db, namespace_root, &record, bytes, &explicit)?;
    let detail = if reclaimed {
        None
    } else {
        let db = db.lock().unwrap();
        records(&db)?
            .into_iter()
            .find(|row| row.id == worktree_id)
            .and_then(|row| row.retained_reason)
    };
    Ok(WorktreeReclaimResult {
        reclaimed,
        bytes_freed: if reclaimed { bytes as i64 } else { 0 },
        disposition: disposition.label().to_owned(),
        detail,
    })
}

/// The checkout a session owns, if it has one. Archiving a chat reclaims *its*
/// worktree — never the workspace's, which belongs to every other chat in it.
pub fn owned_by_session(db: &Connection, session_id: &str) -> Result<Option<WorktreeRecord>, BridgeError> {
    Ok(records(db)?.into_iter().find(|row| {
        row.owner_session_id.as_deref() == Some(session_id) && row.is_candidate()
    }))
}

/// A full maintenance pass, run because a person asked. Same work the tick does.
pub fn run_requested_pass(
    db: &Mutex<Connection>,
    namespace_root: &Path,
    retention: &WorktreeRetention,
) -> Result<SweepOutcome, BridgeError> {
    reconcile(db, namespace_root)?;
    sweep(db, namespace_root, retention)
}

/// Run a reconcile and a sweep, reporting anything a person would want to know.
/// Both producers — boot and the maintenance tick — go through here so neither
/// can discard the outcome.
pub fn run_maintenance_pass(
    db: &Mutex<Connection>,
    namespace_root: &Path,
    retention: &WorktreeRetention,
) {
    match reconcile(db, namespace_root) {
        Ok(outcome) if outcome.is_quiet() => {}
        Ok(outcome) => eprintln!(
            "bridge: worktree reconcile pruned_registrations={} marked_removed={} \
             adopted_orphans={} unverifiable={} external={}",
            outcome.pruned_registrations,
            outcome.marked_removed,
            outcome.adopted_orphans,
            outcome.unverifiable,
            outcome.external,
        ),
        Err(error) => eprintln!("bridge: worktree reconcile failed: {error}"),
    }
    match sweep(db, namespace_root, retention) {
        Ok(outcome) if outcome.is_quiet() => {}
        Ok(outcome) => eprintln!(
            "bridge: worktree sweep removed={} removed_bytes={} retained={} \
             retained_bytes={} over_budget_bytes={} skipped={} measurements_truncated={}",
            outcome.removed,
            outcome.removed_bytes,
            outcome.retained,
            outcome.retained_bytes,
            outcome.over_budget_bytes,
            outcome.skipped,
            outcome.measurements_truncated,
        ),
        Err(error) => eprintln!("bridge: worktree sweep failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::process::Command;

    fn git_cmd(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Mutex<Connection>,
        repo: PathBuf,
        namespace: PathBuf,
        origin: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let origin = dir.path().join("origin.git");
        std::fs::create_dir(&repo).unwrap();
        let repo = std::fs::canonicalize(&repo).unwrap();
        git_cmd(&repo, &["init", "-q", "-b", "main"]);
        git_cmd(&repo, &["config", "user.email", "t@example.invalid"]);
        git_cmd(&repo, &["config", "user.name", "Bridge Test"]);
        git_cmd(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-q", "-m", "base"]);
        git_cmd(&repo, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        git_cmd(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git_cmd(&repo, &["push", "-q", "origin", "main"]);
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![repo.to_string_lossy()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Oslo','Task','main',?1,'ready','now')",
            params![repo.to_string_lossy()],
        )
        .unwrap();
        let namespace = std::fs::canonicalize(dir.path()).unwrap().join("worktrees");
        Fixture {
            _dir: dir,
            db: Mutex::new(db),
            repo,
            namespace,
            origin,
        }
    }

    /// A worker checkout at the layout's own path, plus the row that owns it.
    fn worker_worktree(fixture: &Fixture, name: &str, branch: &str) -> PathBuf {
        let path = fixture
            .namespace
            .join("workers")
            .join("task")
            .join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let base = git_cmd(&fixture.repo, &["rev-parse", "HEAD"]);
        git_cmd(
            &fixture.repo,
            &["worktree", "add", "-q", "-b", branch, path.to_str().unwrap(), "HEAD"],
        );
        register(
            &fixture.db.lock().unwrap(),
            &NewWorktree {
                kind: KIND_WORKER.to_owned(),
                repo_root: fixture.repo.to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                branch: Some(branch.to_owned()),
                owner_session_id: None,
                owner_workspace_id: Some("w".to_owned()),
                base_commit: Some(base),
            },
        )
        .unwrap();
        path
    }

    fn record(fixture: &Fixture, path: &Path) -> WorktreeRecord {
        records(&fixture.db.lock().unwrap())
            .unwrap()
            .into_iter()
            .find(|record| record.path == canonical_key(path))
            .expect("a row for the path")
    }

    fn commit_in(worktree: &Path, name: &str) {
        std::fs::write(worktree.join(name), "change\n").unwrap();
        git_cmd(worktree, &["add", "."]);
        git_cmd(worktree, &["commit", "-q", "-m", name]);
    }

    fn age(fixture: &Fixture, path: &Path, seconds: i64) {
        let when = (Utc::now() - chrono::Duration::seconds(seconds)).to_rfc3339();
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE worktrees SET last_used_at=?2 WHERE path=?1",
                params![canonical_key(path), when],
            )
            .unwrap();
    }

    fn classify_path(fixture: &Fixture, path: &Path) -> Disposition {
        let record = record(fixture, path);
        classify(&fixture.db.lock().unwrap(), &record).unwrap()
    }

    // --- inventory ------------------------------------------------------------

    #[test]
    fn registering_a_worker_worktree_records_its_owner_and_repo() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let record = record(&fixture, &path);
        assert_eq!(record.kind, KIND_WORKER);
        assert_eq!(record.repo_root, fixture.repo.to_string_lossy());
        assert_eq!(record.branch.as_deref(), Some("bridge/task-worker-child"));
        assert_eq!(record.owner_workspace_id.as_deref(), Some("w"));
        assert!(record.base_commit.is_some(), "the base commit is recorded");
        assert_eq!(record.state, STATE_ACTIVE);
    }

    #[test]
    fn re_registering_the_same_path_updates_rather_than_duplicates() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let first = record(&fixture, &path);
        register(
            &fixture.db.lock().unwrap(),
            &NewWorktree {
                kind: KIND_WORKER.to_owned(),
                repo_root: fixture.repo.to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                branch: Some("bridge/task-worker-child".to_owned()),
                owner_session_id: Some("child".to_owned()),
                owner_workspace_id: None,
                base_commit: None,
            },
        )
        .unwrap();
        let rows = records(&fixture.db.lock().unwrap()).unwrap();
        assert_eq!(rows.len(), 1, "one row per path");
        assert_eq!(rows[0].id, first.id, "the same row is updated");
        assert_eq!(rows[0].owner_session_id.as_deref(), Some("child"));
        assert_eq!(
            rows[0].base_commit, first.base_commit,
            "a later registration without a base commit does not erase the recorded one",
        );
    }

    /// An in-place worker records the *user's own checkout* as its worktree, so
    /// a row can name a main working tree. Nothing may put that in front of a
    /// reclaim decision.
    #[test]
    fn a_row_naming_its_own_repository_is_never_a_candidate() {
        let fixture = fixture();
        let repo = fixture.repo.to_string_lossy().to_string();
        register(
            &fixture.db.lock().unwrap(),
            &NewWorktree {
                kind: KIND_WORKER.to_owned(),
                repo_root: repo.clone(),
                path: repo.clone(),
                branch: Some("main".to_owned()),
                owner_session_id: None,
                owner_workspace_id: Some("w".to_owned()),
                base_commit: None,
            },
        )
        .unwrap();
        age(&fixture, &fixture.repo, 400 * 24 * 60 * 60);

        let outcome = sweep(
            &fixture.db,
            &fixture.namespace,
            &WorktreeRetention {
                max_total_bytes: 1,
                max_per_repo: 0,
                ..WorktreeRetention::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.removed, 0, "the repository itself is off the table");
        assert!(fixture.repo.is_dir());
        assert!(
            fixture.repo.join("base.txt").is_file(),
            "the user's checkout is untouched",
        );
    }

    // --- reconcile ------------------------------------------------------------

    #[test]
    fn reconcile_marks_a_row_removed_and_prunes_the_registration_when_the_directory_is_gone() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        std::fs::remove_dir_all(&path).unwrap();
        let outcome = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert_eq!(outcome.marked_removed, 1);
        assert_eq!(outcome.pruned_registrations, 1);
        assert_eq!(record(&fixture, &path).state, STATE_REMOVED);
        assert!(
            git::list_worktrees(&fixture.repo)
                .unwrap()
                .iter()
                .all(|entry| entry.path != path),
            "git no longer reports the registration",
        );
    }

    #[test]
    fn reconcile_adopts_an_untracked_directory_under_the_namespace_root() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        // Drop the row, keeping the checkout: the crash-between-two-steps shape.
        fixture
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM worktrees", [])
            .unwrap();
        let outcome = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert_eq!(outcome.adopted_orphans, 1);
        assert_eq!(outcome.unverifiable, 0);
        let adopted = record(&fixture, &path);
        assert_eq!(adopted.state, STATE_ORPHANED);
        assert_eq!(adopted.kind, KIND_WORKER);
        assert_eq!(adopted.repo_root, fixture.repo.to_string_lossy());
        assert_eq!(adopted.branch.as_deref(), Some("bridge/task-worker-child"));
    }

    /// An orchestrator checkout nests per workspace *and* per session, so it
    /// sits at the same depth as a worker's. Walking one level finds the
    /// workspace grouping directory, which is not a worktree at all.
    #[test]
    fn reconcile_adopts_an_orchestrator_checkout_at_its_own_depth() {
        let fixture = fixture();
        let path = fixture
            .namespace
            .join("orchestrators")
            .join("task")
            .join("session-7");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        git_cmd(
            &fixture.repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "bridge/task-session7",
                path.to_str().unwrap(),
                "HEAD",
            ],
        );

        let outcome = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert_eq!(outcome.adopted_orphans, 1);
        assert_eq!(outcome.external, 0, "it is Bridge's own checkout");
        let adopted = record(&fixture, &path);
        assert_eq!(adopted.kind, KIND_ORCHESTRATOR);
        assert_eq!(adopted.state, STATE_ORPHANED);
        assert!(
            records(&fixture.db.lock().unwrap())
                .unwrap()
                .iter()
                .all(|row| row.path != canonical_key(path.parent().unwrap())),
            "the workspace grouping directory is not itself a worktree",
        );
    }

    /// The shape observed live: the checkout survives but the administrative
    /// directory backing it is gone, so git answers every question with an
    /// error. Nothing about it can be proven, so nothing may be done to it.
    #[test]
    fn reconcile_records_a_directory_git_cannot_read_as_unverifiable() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        fixture
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM worktrees", [])
            .unwrap();
        let admin = fixture
            .repo
            .join(".git")
            .join("worktrees")
            .join("child");
        std::fs::remove_dir_all(&admin).unwrap();

        let outcome = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert_eq!(outcome.unverifiable, 1);
        let adopted = record(&fixture, &path);
        assert_eq!(adopted.state, STATE_UNVERIFIABLE);
        assert!(path.is_dir(), "the directory is reported, never removed");
        assert_eq!(
            adopted.repo_root,
            fixture.repo.to_string_lossy(),
            "the .git pointer still names the repository, so it stays attributable",
        );

        assert!(matches!(
            classify_path(&fixture, &path),
            Disposition::Unverifiable(_)
        ));
        let swept = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(swept.removed, 0, "an unverifiable checkout is never swept");
        assert!(path.is_dir());
    }

    #[test]
    fn reconcile_leaves_external_worktrees_alone() {
        let fixture = fixture();
        worker_worktree(&fixture, "child", "bridge/task-worker-child");
        // A developer's own worktree of the same repository, outside Bridge's
        // namespace entirely.
        let outside = fixture._dir.path().join("hand-made");
        git_cmd(
            &fixture.repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "chore/hand-made",
                outside.to_str().unwrap(),
                "HEAD",
            ],
        );
        let outcome = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert_eq!(outcome.external, 1);
        let external = record(&fixture, &outside);
        assert_eq!(external.state, STATE_EXTERNAL);
        assert_eq!(external.kind, KIND_EXTERNAL, "not miscast as Bridge's own");

        let swept = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(swept.removed, 0);
        assert!(outside.is_dir(), "someone else's worktree is never removed");
    }

    #[test]
    fn reconcile_is_idempotent() {
        let fixture = fixture();
        worker_worktree(&fixture, "child", "bridge/task-worker-child");
        reconcile(&fixture.db, &fixture.namespace).unwrap();
        let before = records(&fixture.db.lock().unwrap()).unwrap();
        let second = reconcile(&fixture.db, &fixture.namespace).unwrap();
        assert!(second.is_quiet(), "a settled inventory reports nothing: {second:?}");
        assert_eq!(before, records(&fixture.db.lock().unwrap()).unwrap());
    }

    // --- classification -------------------------------------------------------

    #[test]
    fn a_clean_worktree_with_no_commits_past_base_is_reclaimable() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        assert_eq!(classify_path(&fixture, &path), Disposition::Reclaimable);
    }

    #[test]
    fn a_dirty_worktree_is_at_risk() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        std::fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();
        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::AtRisk(reason) if reason.contains("uncommitted")),
            "{disposition:?}",
        );
    }

    #[test]
    fn a_worktree_with_local_only_commits_is_at_risk() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "only-here.txt");
        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::AtRisk(reason) if reason.contains("only in this checkout")),
            "{disposition:?}",
        );
    }

    #[test]
    fn a_clean_worktree_whose_commits_are_pushed_is_pushed_unmerged() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "shared.txt");
        git_cmd(&path, &["push", "-q", "origin", "bridge/task-worker-child"]);
        assert_eq!(classify_path(&fixture, &path), Disposition::PushedUnmerged);
        assert!(fixture.origin.is_dir());
    }

    #[test]
    fn a_worktree_a_live_session_runs_in_is_retained() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,depth)
                 VALUES('live','w','codex','Live','working','reported',?1,0)",
                params![path.to_string_lossy()],
            )
            .unwrap();
        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::Retained(reason) if reason.contains("live")),
            "{disposition:?}",
        );
    }

    /// The borrowed-verifier case: a different session runs in the worker's
    /// checkout under its own runtime row.
    #[test]
    fn a_worktree_a_live_worker_is_bound_to_is_retained() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let db = fixture.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth)
             VALUES('parent','w','codex','Parent','ready','reported',0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth)
             VALUES('verifier','w','claude','Verifier','working','reported',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_runtime(
                session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,
                worktree_path,updated_at)
             VALUES('verifier','parent','working','verification','claude',?1,'now')",
            params![path.to_string_lossy()],
        )
        .unwrap();
        drop(db);
        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::Retained(reason) if reason.contains("verifier")),
            "{disposition:?}",
        );
    }

    #[test]
    fn a_pending_adoption_worktree_with_real_changes_is_retained() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "worker-output.txt");
        record_pending_adoption(&fixture, &path, "bridge/task-worker-child");
        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::Retained(reason) if reason.contains("adopted")),
            "{disposition:?}",
        );
    }

    #[test]
    fn a_pending_adoption_worktree_with_an_empty_diff_is_reclaimable() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        record_pending_adoption(&fixture, &path, "bridge/task-worker-child");
        assert_eq!(
            classify_path(&fixture, &path),
            Disposition::Reclaimable,
            "there is nothing here for anyone to adopt",
        );
    }

    /// Mirrors what `record_binding` writes in production, `base_branch`
    /// included — the branch-release guard reconstructs the expected worker
    /// branch name from it, so a fixture that omitted it made the guard look
    /// broken rather than strict.
    fn record_pending_adoption(fixture: &Fixture, path: &Path, branch: &str) {
        let db = fixture.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth)
             VALUES('child','w','claude','Worker','stopped','reported',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_worktree_adoptions(
                session_id,parent_session_id,workspace_id,worktree_path,worktree_branch,
                task_worktree_path,state,base_branch,created_at,updated_at)
             VALUES('child','child','w',?1,?3,?2,'pending_adoption','main','now','now')",
            params![path.to_string_lossy(), fixture.repo.to_string_lossy(), branch],
        )
        .unwrap();
    }

    // --- sweep ----------------------------------------------------------------

    /// A TTL has to measure idleness. Nothing calls a touch on every turn, so
    /// the sweep derives last use from the owning session's own activity —
    /// without which a chat someone works in daily would age out of its TTL and
    /// have its checkout collected under them.
    #[test]
    fn recent_activity_on_the_owning_session_keeps_its_checkout_alive() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        {
            let db = fixture.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth)
                 VALUES('child','w','claude','Worker','completed','reported',1)",
                [],
            )
            .unwrap();
            db.execute(
                "UPDATE worktrees SET owner_session_id='child' WHERE path=?1",
                params![canonical_key(&path)],
            )
            .unwrap();
            store::event(&db, "supervisor", "session.turn", "child", "worked just now").unwrap();
        }
        // Old enough to collect on its recorded timestamp alone.
        age(&fixture, &path, 30 * 24 * 60 * 60);

        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 0, "the session's own activity is newer");
        assert!(path.is_dir());
        assert!(
            record(&fixture, &path).idle_seconds() < 60,
            "idleness is measured from the last recorded activity",
        );
    }

    #[test]
    fn the_sweep_reclaims_a_reclaimable_worktree_past_its_ttl() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        age(&fixture, &path, 2 * 24 * 60 * 60);
        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 1);
        assert!(!path.exists());
        assert_eq!(record(&fixture, &path).state, STATE_REMOVED);
    }

    #[test]
    fn the_sweep_leaves_a_reclaimable_worktree_inside_its_ttl() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 0);
        assert!(path.is_dir());
        assert_eq!(
            record(&fixture, &path).disposition.as_deref(),
            Some("reclaimable"),
            "the assessment is recorded even when the TTL is not reached",
        );
    }

    #[test]
    fn the_sweep_never_removes_an_at_risk_worktree_however_old() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        std::fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();
        age(&fixture, &path, 400 * 24 * 60 * 60);
        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 0);
        assert_eq!(outcome.retained, 1);
        assert!(path.is_dir());
        let row = record(&fixture, &path);
        assert_eq!(row.disposition.as_deref(), Some("at_risk"));
        assert!(row.retained_reason.unwrap().contains("uncommitted"));
    }

    #[test]
    /// A cap has to leave room for the work it is blocking. The creation gate
    /// refuses at `>= max_per_repo`, so a sweep that stopped at equality left
    /// the repository permanently full: nothing was reclaimed and every queued
    /// delegation waited on a TTL that had no reason to expire.
    fn a_repository_exactly_at_its_count_cap_frees_one_slot_oldest_first() {
        let fixture = fixture();
        let oldest = worker_worktree(&fixture, "oldest", "bridge/task-worker-oldest");
        let middle = worker_worktree(&fixture, "middle", "bridge/task-worker-middle");
        let newest = worker_worktree(&fixture, "newest", "bridge/task-worker-newest");
        // All well inside the worker TTL: only the cap can justify a removal.
        age(&fixture, &oldest, 900);
        age(&fixture, &middle, 600);
        age(&fixture, &newest, 60);
        let repo = fixture.repo.to_string_lossy().to_string();
        let retention = WorktreeRetention {
            max_per_repo: 3,
            ..WorktreeRetention::default()
        };
        assert!(
            !has_capacity(&fixture.db.lock().unwrap(), &repo, &retention).unwrap(),
            "the gate refuses at the cap, which is what the sweep has to clear",
        );

        let outcome = sweep(&fixture.db, &fixture.namespace, &retention).unwrap();
        assert_eq!(outcome.removed, 1, "only as many as the cap requires");
        assert!(!oldest.exists(), "the least recently used goes first");
        assert!(middle.is_dir());
        assert!(newest.is_dir());
        assert!(
            has_capacity(&fixture.db.lock().unwrap(), &repo, &retention).unwrap(),
            "the queued delegation can now proceed",
        );
    }

    #[test]
    fn exceeding_the_byte_cap_evicts_reclaimable_worktrees_inside_their_ttl() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        // Well inside the TTL, so only the byte cap can justify this.
        age(&fixture, &path, 30);
        let retention = WorktreeRetention {
            max_total_bytes: 1,
            ..WorktreeRetention::default()
        };
        let outcome = sweep(&fixture.db, &fixture.namespace, &retention).unwrap();
        assert_eq!(outcome.removed, 1);
        assert!(!path.exists());
    }

    #[test]
    fn a_cap_breach_only_at_risk_worktrees_could_satisfy_is_reported_not_forced() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        std::fs::write(path.join("scratch.txt"), "unsaved\n").unwrap();
        let retention = WorktreeRetention {
            max_total_bytes: 1,
            ..WorktreeRetention::default()
        };
        let outcome = sweep(&fixture.db, &fixture.namespace, &retention).unwrap();
        assert_eq!(outcome.removed, 0, "a cap never overrides safety");
        assert!(path.is_dir());
        assert!(
            outcome.over_budget_bytes > 0,
            "the breach is reported instead: {outcome:?}",
        );
    }

    #[test]
    fn the_pushed_unmerged_policy_is_honored() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "shared.txt");
        git_cmd(&path, &["push", "-q", "origin", "bridge/task-worker-child"]);
        age(&fixture, &path, 2 * 24 * 60 * 60);

        let retained = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(retained.removed, 0, "retain is the default");
        assert!(path.is_dir());

        let outcome = sweep(
            &fixture.db,
            &fixture.namespace,
            &WorktreeRetention {
                pushed_unmerged: PushedUnmergedPolicy::Delete,
                ..WorktreeRetention::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.removed, 1);
        assert!(!path.exists());
        assert_eq!(
            git_cmd(&fixture.repo, &["branch", "--list", "--format=%(refname:short)", "bridge/task-worker-child"]),
            "bridge/task-worker-child",
            "the branch ref survives; only the checkout is reclaimed",
        );
    }

    /// Deletion is best effort per checkout. One that git refuses — here because
    /// its recorded repository root no longer resolves — is counted and skipped,
    /// and the rest of the pass still runs.
    #[test]
    fn a_removal_that_fails_does_not_stall_the_rest_of_the_sweep() {
        let fixture = fixture();
        let broken = worker_worktree(&fixture, "broken", "bridge/task-worker-broken");
        let healthy = worker_worktree(&fixture, "healthy", "bridge/task-worker-healthy");
        age(&fixture, &broken, 2 * 24 * 60 * 60);
        age(&fixture, &healthy, 2 * 24 * 60 * 60);
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE worktrees SET repo_root='/nonexistent/repo' WHERE path=?1",
                params![canonical_key(&broken)],
            )
            .unwrap();

        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.removed, 1);
        assert!(broken.is_dir(), "the refusal leaves it in place");
        assert!(!healthy.exists(), "the rest of the sweep still ran");
        let retained = record(&fixture, &broken).retained_reason.unwrap();
        assert!(!retained.is_empty(), "the refusal is recorded");
        assert_eq!(
            outcome.retained, 1,
            "a checkout that survived a refused removal is retained, not written off",
        );

        // The bytes it still occupies stay on the books: planning a removal
        // must not be able to make a breach disappear.
        let breached = sweep(
            &fixture.db,
            &fixture.namespace,
            &WorktreeRetention {
                max_total_bytes: 1,
                ..WorktreeRetention::default()
            },
        )
        .unwrap();
        assert!(
            breached.over_budget_bytes > 0,
            "the refused checkout still owes its bytes: {breached:?}",
        );
    }

    #[test]
    fn every_reclaim_writes_an_audit_event_naming_what_it_freed() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        age(&fixture, &path, 2 * 24 * 60 * 60);
        sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        let body: String = fixture
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT body FROM events WHERE kind='worktree.reclaimed' AND entity_id=?1",
                params![canonical_key(&path)],
                |row| row.get(0),
            )
            .unwrap();
        assert!(body.contains("bridge/task-worker-child"), "{body}");
        assert!(body.contains("reclaimable"), "{body}");
        assert!(body.contains("freed"), "{body}");
    }

    // --- review regressions ---------------------------------------------------

    /// A chat whose provider is up but has no turn in flight sits at `ready`,
    /// not `working`. A status-only liveness test called that idle and deleted
    /// the directory out from under a live adapter.
    #[test]
    fn a_ready_chat_holding_a_live_adapter_keeps_its_checkout() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "chat", "bridge/task-chat");
        {
            let db = fixture.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,depth,adapter_pid,adapter_process_identity)
                 VALUES('chat','w','claude','Chat','ready','reported',?1,0,4242,'claude:4242')",
                params![canonical_key(&path)],
            )
            .unwrap();
        }
        age(&fixture, &path, 400 * 24 * 60 * 60);

        let disposition = classify_path(&fixture, &path);
        assert!(
            matches!(&disposition, Disposition::Retained(reason) if reason.contains("chat")),
            "{disposition:?}",
        );
        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 0);
        assert!(path.is_dir(), "the live adapter keeps its working directory");
    }

    /// Boot recovery clears `adapter_pid` for every process that is really
    /// gone, so a `ready` row left by a crashed run must not pin a checkout.
    #[test]
    fn a_ready_session_with_no_process_claim_does_not_pin_its_checkout() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "chat", "bridge/task-chat");
        {
            let db = fixture.db.lock().unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,depth)
                 VALUES('chat','w','claude','Chat','ready','reported',?1,0)",
                params![canonical_key(&path)],
            )
            .unwrap();
        }
        age(&fixture, &path, 400 * 24 * 60 * 60);
        assert_eq!(classify_path(&fixture, &path), Disposition::Reclaimable);
    }

    /// Reclaiming a checkout must not cost the session that owns it: the branch
    /// survives, so the checkout can be cut again on resume.
    #[test]
    fn a_reclaimed_checkout_is_restored_from_its_recorded_branch() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "chat", "bridge/task-chat");
        age(&fixture, &path, 2 * 24 * 60 * 60);
        assert_eq!(
            sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default())
                .unwrap()
                .removed,
            1,
        );
        assert!(!path.exists());

        assert!(restore_if_reclaimed(&fixture.db, &path));
        assert!(path.is_dir(), "the checkout is back");
        assert_eq!(git::current_branch(&path).as_deref(), Some("bridge/task-chat"));
        assert!(
            path.join("base.txt").is_file(),
            "and it is the project, not an empty directory",
        );
        assert_eq!(record(&fixture, &path).state, STATE_ACTIVE);
        assert!(
            !restore_if_reclaimed(&fixture.db, &path),
            "restoring an existing checkout is a no-op",
        );
    }

    #[test]
    fn restore_never_recreates_a_checkout_bridge_did_not_cut() {
        let fixture = fixture();
        let outside = fixture._dir.path().join("hand-made");
        git_cmd(
            &fixture.repo,
            &["worktree", "add", "-q", "-b", "chore/hand-made", outside.to_str().unwrap(), "HEAD"],
        );
        reconcile(&fixture.db, &fixture.namespace).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
        assert!(!restore_if_reclaimed(&fixture.db, &outside));
        assert!(!outside.exists());
    }

    /// `safe_remove_worker_worktree` only re-runs `git status`, and
    /// `git worktree remove` does not need force for a clean tree carrying a
    /// commit that exists nowhere else. The removal path therefore has to
    /// re-decide the whole classification for itself.
    #[test]
    fn the_removal_path_re_decides_before_deleting() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let planned = record(&fixture, &path);
        assert_eq!(classify_path(&fixture, &path), Disposition::Reclaimable);

        // The world moves between the scan and the deletion.
        commit_in(&path, "arrived-after-the-scan.txt");

        let removed = remove_recorded_worktree(
            &fixture.db,
            &fixture.namespace,
            &planned,
            0,
            &WorktreeRetention::default(),
        )
        .unwrap();
        assert!(!removed, "a stale plan does not authorize a deletion");
        assert!(path.is_dir());
        assert_eq!(
            std::fs::read_to_string(path.join("arrived-after-the-scan.txt")).unwrap(),
            "change\n",
            "the commit that arrived late is still there",
        );
    }

    #[test]
    fn a_new_measurement_replaces_a_stale_larger_one() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE worktrees SET size_bytes=?2 WHERE path=?1",
                params![canonical_key(&path), 11 * 1024 * 1024 * 1024i64],
            )
            .unwrap();
        let repo = fixture.repo.to_string_lossy().to_string();
        assert!(
            !has_capacity(
                &fixture.db.lock().unwrap(),
                &repo,
                &WorktreeRetention::default()
            )
            .unwrap(),
            "the stale size alone exhausts the byte budget",
        );

        sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        let measured = record(&fixture, &path).size_bytes.unwrap();
        assert!(
            measured < 11 * 1024 * 1024 * 1024,
            "the freed space is reflected: {measured}",
        );
        assert!(
            has_capacity(
                &fixture.db.lock().unwrap(),
                &repo,
                &WorktreeRetention::default()
            )
            .unwrap(),
            "and creation is allowed again",
        );
    }

    /// An unsettled binding that outlives its checkout keeps offering the user
    /// an adopt-or-discard decision about a directory that no longer exists,
    /// and the worker-side collector skips it because the path is missing.
    #[test]
    fn reclaiming_an_empty_unadopted_checkout_settles_its_binding() {
        let fixture = fixture();
        // Named the way `prepare_isolated_worker` names it, so the branch guard
        // recognises it as Bridge's own.
        let path = worker_worktree(&fixture, "child", "main-worker-child");
        record_pending_adoption(&fixture, &path, "main-worker-child");
        age(&fixture, &path, 2 * 24 * 60 * 60);

        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 1);
        let state: String = fixture
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM worker_worktree_adoptions WHERE session_id='child'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            state, "empty",
            "no pending decision is left pointing at a deleted directory",
        );
        assert_eq!(
            git_cmd(&fixture.repo, &["branch", "--list", "--format=%(refname:short)", "main-worker-child"]),
            "",
            "and the scratch ref goes with it, as settlement would have done",
        );
    }

    /// A symlink dropped into a layout slot — which anything with write access
    /// to its own checkout can create as a sibling — must not become an
    /// inventoried Bridge checkout at its resolved path.
    #[test]
    fn a_symlink_in_a_layout_slot_is_never_adopted_or_reclaimed() {
        let fixture = fixture();
        let outside = fixture._dir.path().join("someone-elses");
        git_cmd(
            &fixture.repo,
            &["worktree", "add", "-q", "-b", "chore/theirs", outside.to_str().unwrap(), "HEAD"],
        );
        let slot = fixture.namespace.join("github").join("pr-1-theirs");
        std::fs::create_dir_all(slot.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &slot).unwrap();

        reconcile(&fixture.db, &fixture.namespace).unwrap();
        let rows = records(&fixture.db.lock().unwrap()).unwrap();
        assert!(
            rows.iter().all(|row| row.state != STATE_ORPHANED),
            "the symlink is not adopted as a Bridge checkout: {rows:?}",
        );
        for row in &rows {
            if canonical_key(Path::new(&row.path)) == canonical_key(&outside) {
                assert_eq!(row.state, STATE_EXTERNAL, "and if seen at all, it is external");
            }
        }

        let outcome = sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(outcome.removed, 0);
        assert!(outside.is_dir(), "somebody else's worktree survives");
        assert!(outside.join("base.txt").is_file());
    }

    // --- explicit reclaim -----------------------------------------------------

    #[test]
    fn reclaim_refuses_a_checkout_bridge_did_not_create() {
        let fixture = fixture();
        let outside = fixture._dir.path().join("hand-made");
        git_cmd(
            &fixture.repo,
            &["worktree", "add", "-q", "-b", "chore/hand-made", outside.to_str().unwrap(), "HEAD"],
        );
        reconcile(&fixture.db, &fixture.namespace).unwrap();
        let id = record(&fixture, &outside).id;

        let outcome = reclaim(
            &fixture.db,
            &fixture.namespace,
            &id,
            &WorktreeRetention::default(),
        )
        .unwrap();
        assert!(!outcome.reclaimed);
        assert!(
            outcome.detail.unwrap().contains("not one Bridge created"),
            "the refusal says why",
        );
        assert!(outside.is_dir());
    }

    #[test]
    fn reclaim_refuses_work_that_exists_nowhere_else_and_says_why() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "only-here.txt");
        let id = record(&fixture, &path).id;

        let outcome = reclaim(
            &fixture.db,
            &fixture.namespace,
            &id,
            &WorktreeRetention::default(),
        )
        .unwrap();
        assert!(!outcome.reclaimed);
        assert_eq!(outcome.disposition, "at_risk");
        assert!(
            outcome.detail.unwrap().contains("only in this checkout"),
            "and it is a result, not an error",
        );
        assert!(path.is_dir());
    }

    /// The difference between the sweep and a person: the sweep retains a
    /// checkout whose commits are all on a remote, and a person who has been
    /// shown that may discard it.
    #[test]
    fn an_explicit_reclaim_may_take_pushed_work_the_sweep_retains() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        commit_in(&path, "shared.txt");
        git_cmd(&path, &["push", "-q", "origin", "bridge/task-worker-child"]);
        age(&fixture, &path, 2 * 24 * 60 * 60);
        let id = record(&fixture, &path).id;

        assert_eq!(
            sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default())
                .unwrap()
                .removed,
            0,
            "the unattended sweep leaves it alone",
        );
        let outcome = reclaim(
            &fixture.db,
            &fixture.namespace,
            &id,
            &WorktreeRetention::default(),
        )
        .unwrap();
        assert!(outcome.reclaimed, "{outcome:?}");
        assert!(outcome.bytes_freed > 0);
        assert!(!path.exists());
    }

    #[test]
    fn reclaiming_an_unrecorded_id_is_an_error_not_a_silent_success() {
        let fixture = fixture();
        let error = reclaim(
            &fixture.db,
            &fixture.namespace,
            "no-such-id",
            &WorktreeRetention::default(),
        )
        .unwrap_err();
        assert!(matches!(error, BridgeError::Invalid(_)), "{error:?}");
    }

    #[test]
    fn a_requested_pass_reconciles_before_it_sweeps() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        // No row at all: only the reconcile half can find this.
        fixture
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM worktrees", [])
            .unwrap();

        run_requested_pass(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        assert_eq!(
            record(&fixture, &path).state,
            STATE_ORPHANED,
            "the pass adopted it before deciding anything about it",
        );
    }

    // --- capacity -------------------------------------------------------------

    #[test]
    fn capacity_refuses_once_the_count_cap_is_reached() {
        let fixture = fixture();
        worker_worktree(&fixture, "child", "bridge/task-worker-child");
        let repo = fixture.repo.to_string_lossy().to_string();
        let retention = WorktreeRetention {
            max_per_repo: 1,
            ..WorktreeRetention::default()
        };
        let db = fixture.db.lock().unwrap();
        assert!(!has_capacity(&db, &repo, &retention).unwrap());
        let reason = over_capacity(&db, &repo, &retention).unwrap().unwrap();
        assert!(reason.contains("limit 1"), "{reason}");
        assert!(
            has_capacity(&db, &repo, &WorktreeRetention::default()).unwrap(),
            "the default budget still has room",
        );
    }

    #[test]
    fn usage_reports_totals_against_the_caps_in_force() {
        let fixture = fixture();
        let path = worker_worktree(&fixture, "child", "bridge/task-worker-child");
        age(&fixture, &path, 30);
        sweep(&fixture.db, &fixture.namespace, &WorktreeRetention::default()).unwrap();
        let usage = usage(
            &fixture.db.lock().unwrap(),
            &WorktreeRetention::default(),
        )
        .unwrap();
        assert_eq!(usage.total_count, 1);
        assert_eq!(usage.reclaimable_count, 1);
        assert!(usage.total_bytes > 0, "the sweep measured it");
        assert_eq!(usage.max_per_repo, 12);
        assert_eq!(usage.repositories.len(), 1);
        assert!(!usage.repositories[0].over_budget);
    }

    #[test]
    fn human_bytes_reads_as_a_size() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }
}
