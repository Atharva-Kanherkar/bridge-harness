//! Durable lifecycle for a worker's repository output.
//!
//! An `Isolated` worker writes into its own child worktree. Verifying that
//! worktree proves nothing about the user's task checkout: without an explicit
//! adoption step, a "verified" change can live only in a child worktree that is
//! then cleaned up, leaving the task workspace untouched. Every non-read-only
//! worker therefore gets a durable binding row at launch recording *where* it
//! writes, and that row carries the adoption state afterwards:
//!
//! ```text
//! in_place                     shared/full writer — nothing to adopt
//! pending_adoption ─┬─ adopted     merged into the task worktree
//!                   └─ discarded   thrown away on purpose
//! empty                        isolated writer that produced no change
//! ```
//!
//! A parent session is not allowed to finish while any of its children sit in
//! `pending_adoption`, and a child worktree is removed only once its row is
//! terminal. Restart recovery reconciles rows whose worktree no longer exists.

use crate::{
    delegation::{WorkerResult, WorkerResultStatus},
    git, store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Worker writes directly into the task worktree; there is nothing to adopt.
pub const STATE_IN_PLACE: &str = "in_place";
/// Isolated worker produced changes that exist only in its own worktree.
pub const STATE_PENDING: &str = "pending_adoption";
/// Changes were integrated into the task worktree.
pub const STATE_ADOPTED: &str = "adopted";
/// Changes were deliberately thrown away.
pub const STATE_DISCARDED: &str = "discarded";
/// Isolated worker finished without touching the repository.
pub const STATE_EMPTY: &str = "empty";
/// Claimed by an in-flight adopt or discard. Git runs with the database lock
/// released, so the claim is what stops a second call from merging and then
/// recording a discard — or removing the worktree the first call is merging from.
pub const STATE_SETTLING: &str = "settling";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRepositoryBinding {
    pub session_id: String,
    pub parent_session_id: String,
    pub workspace_id: String,
    /// The exact directory the worker runs in — a child worktree for isolated
    /// workers, the task worktree otherwise. Never null, so parent-facing
    /// evidence can always name the checkout the claim came from.
    pub worktree_path: String,
    pub worktree_branch: String,
    pub task_worktree_path: String,
    pub state: String,
    pub head: Option<String>,
    pub base_commit: Option<String>,
    /// Branch the work is based on — the *task* checkout's branch, not the
    /// worker's own. This is what "relative to" means in completion evidence.
    pub base_branch: Option<String>,
    /// Paths already dirty in the checkout when the worker launched. Only
    /// meaningful for an in-place writer, which shares the tree with the user and
    /// with sibling workers; without it their edits would be attributed to it.
    pub baseline_dirty_paths: Vec<String>,
    pub changed_paths: Vec<String>,
    pub diffstat: Option<String>,
    pub dirty: bool,
    pub detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl WorkerRepositoryBinding {
    pub fn is_isolated(&self) -> bool {
        self.worktree_path != self.task_worktree_path
    }

    pub fn awaits_adoption(&self) -> bool {
        self.state == STATE_PENDING
    }

    /// Terminal states are the only ones where removing the child worktree is
    /// safe: anything else could destroy work the user has not decided about.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state.as_str(),
            STATE_ADOPTED | STATE_DISCARDED | STATE_EMPTY | STATE_IN_PLACE
        )
    }
}

fn map_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkerRepositoryBinding> {
    Ok(WorkerRepositoryBinding {
        session_id: row.get(0)?,
        parent_session_id: row.get(1)?,
        workspace_id: row.get(2)?,
        worktree_path: row.get(3)?,
        worktree_branch: row.get(4)?,
        task_worktree_path: row.get(5)?,
        state: row.get(6)?,
        head: row.get(7)?,
        base_commit: row.get(8)?,
        base_branch: row.get(9)?,
        baseline_dirty_paths: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
        changed_paths: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
        diffstat: row.get(12)?,
        dirty: row.get(13)?,
        detail: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

const SELECT: &str =
    "SELECT session_id,parent_session_id,workspace_id,worktree_path,worktree_branch,
     task_worktree_path,state,head,base_commit,base_branch,baseline_dirty_paths,changed_paths,
     diffstat,dirty,detail,created_at,updated_at
     FROM worker_worktree_adoptions";

/// Record where a worker writes, at launch. Called for every non-read-only
/// worker so a claim can always be checked against a known checkout, and so an
/// isolated worker's base revision is known before it changes anything.
#[allow(clippy::too_many_arguments)]
pub fn record_binding(
    db: &Connection,
    session_id: &str,
    parent_session_id: &str,
    workspace_id: &str,
    worktree_path: &str,
    worktree_branch: &str,
    task_worktree_path: &str,
    isolated: bool,
) -> Result<(), BridgeError> {
    if worktree_path.trim().is_empty() || task_worktree_path.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "a worker repository binding requires both the worker and task worktree paths".into(),
        ));
    }
    let launch_state = git::derive_repository_evidence(Path::new(worktree_path), None).ok();
    let base_commit = launch_state.as_ref().map(|evidence| evidence.head.clone());
    // An in-place writer shares the checkout with the user and with siblings, so
    // whatever is already dirty at launch is not its doing.
    let baseline_dirty = if isolated {
        Vec::new()
    } else {
        launch_state
            .as_ref()
            .map(|evidence| evidence.dirty_paths.clone())
            .unwrap_or_default()
    };
    let base_branch = git::current_branch(Path::new(task_worktree_path));
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO worker_worktree_adoptions(session_id,parent_session_id,workspace_id,worktree_path,worktree_branch,task_worktree_path,state,base_commit,base_branch,baseline_dirty_paths,changed_paths,dirty,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'[]',0,?11,?11)
         ON CONFLICT(session_id) DO UPDATE SET
            worktree_path=excluded.worktree_path,
            worktree_branch=excluded.worktree_branch,
            task_worktree_path=excluded.task_worktree_path,
            -- Relaunching a warm worker must never re-open a decision the user
            -- already made about its previous output.
            state=CASE WHEN worker_worktree_adoptions.state IN ('adopted','discarded')
                       THEN worker_worktree_adoptions.state ELSE excluded.state END,
            base_commit=COALESCE(worker_worktree_adoptions.base_commit,excluded.base_commit),
            base_branch=COALESCE(worker_worktree_adoptions.base_branch,excluded.base_branch),
            baseline_dirty_paths=COALESCE(worker_worktree_adoptions.baseline_dirty_paths,excluded.baseline_dirty_paths),
            updated_at=excluded.updated_at",
        params![
            session_id,
            parent_session_id,
            workspace_id,
            worktree_path,
            worktree_branch,
            task_worktree_path,
            if isolated { STATE_PENDING } else { STATE_IN_PLACE },
            base_commit,
            base_branch,
            serde_json::to_string(&baseline_dirty)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?,
            now,
        ],
    )?;
    Ok(())
}

pub fn binding(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WorkerRepositoryBinding>, BridgeError> {
    db.query_row(
        &format!("{SELECT} WHERE session_id=?1"),
        params![session_id],
        map_binding,
    )
    .optional()
    .map_err(BridgeError::from)
}

/// Attach the repository evidence a worker actually produced and settle the
/// state: an isolated worker with changes needs adoption, one without does not.
pub fn record_evidence(
    db: &Connection,
    session_id: &str,
    evidence: &git::RepositoryEvidence,
) -> Result<Option<WorkerRepositoryBinding>, BridgeError> {
    let Some(existing) = binding(db, session_id)? else {
        return Ok(None);
    };
    // Never re-open a decision the user already made.
    if matches!(existing.state.as_str(), STATE_ADOPTED | STATE_DISCARDED) {
        return Ok(Some(existing));
    }
    let attributable = evidence
        .changed_paths()
        .into_iter()
        .filter(|path| !existing.baseline_dirty_paths.contains(path))
        .count();
    let state = if !existing.is_isolated() {
        STATE_IN_PLACE
    } else if evidence.commits.is_empty() && attributable == 0 {
        STATE_EMPTY
    } else {
        STATE_PENDING
    };
    let changed = evidence
        .changed_paths()
        .into_iter()
        .filter(|path| !existing.baseline_dirty_paths.contains(path))
        .collect::<Vec<_>>();
    db.execute(
        "UPDATE worker_worktree_adoptions SET state=?2,head=?3,changed_paths=?4,diffstat=?5,dirty=?6,updated_at=?7 WHERE session_id=?1",
        params![
            session_id,
            state,
            evidence.head,
            serde_json::to_string(&changed)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?,
            evidence.diffstat(),
            evidence.dirty(),
            Utc::now().to_rfc3339(),
        ],
    )?;
    binding(db, session_id)
}

/// A worker's typed result checked against what the repository actually shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledResult {
    /// The result to report. Identical to the worker's when the evidence backs
    /// it up; downgraded, with `filesChanged` replaced by derived paths, when it
    /// does not.
    pub result: WorkerResult,
    pub evidence: Option<git::RepositoryEvidence>,
    pub binding: Option<WorkerRepositoryBinding>,
    /// Non-empty when the claim and the repository disagree.
    pub mismatches: Vec<String>,
}

/// Check a worker's claim against Git before it is accepted as the canonical
/// result.
///
/// A `completed` write-mode result used to be trusted as-is: schema validation
/// only. That let a worker report five changed files and two new test files
/// against a repository that was clean, and the completion planner then chose
/// its required checks from that prose. Everything here is derived from the
/// checkout the worker was bound to at launch.
pub fn reconcile_result_with_repository(
    db: &Connection,
    session_id: &str,
    result: &WorkerResult,
) -> ReconciledResult {
    let Ok(Some(binding_row)) = binding(db, session_id) else {
        // Read-only workers have no binding; their guard is the read-only sandbox.
        return ReconciledResult {
            result: result.clone(),
            evidence: None,
            binding: None,
            mismatches: Vec::new(),
        };
    };
    let evidence = git::derive_repository_evidence(
        Path::new(&binding_row.worktree_path),
        binding_row.base_commit.as_deref(),
    );
    reconcile_with_derived_evidence(db, session_id, result, binding_row, evidence)
}

/// The database half of [`reconcile_result_with_repository`], taking evidence the
/// caller already derived. Split so a caller holding no lock can run Git first and
/// only take the lock to record the outcome.
pub fn reconcile_with_derived_evidence(
    db: &Connection,
    session_id: &str,
    result: &WorkerResult,
    binding_row: WorkerRepositoryBinding,
    evidence: Result<git::RepositoryEvidence, BridgeError>,
) -> ReconciledResult {
    let Ok(evidence) = evidence else {
        let mut downgraded = result.clone();
        let detail = format!(
            "Bridge could not read the worker's repository at {} to verify this result",
            binding_row.worktree_path
        );
        if downgraded.status == WorkerResultStatus::Completed {
            downgraded.status = WorkerResultStatus::Blocked;
        }
        downgraded.risks.push(detail.clone());
        return ReconciledResult {
            result: downgraded,
            evidence: None,
            binding: Some(binding_row),
            mismatches: vec![detail],
        };
    };
    let recorded = record_evidence(db, session_id, &evidence)
        .ok()
        .flatten()
        .unwrap_or(binding_row);
    // An in-place writer shares the checkout, so paths that were already dirty
    // when it launched are not its changes and must not be charged to it.
    let derived = evidence
        .changed_paths()
        .into_iter()
        .filter(|path| !recorded.baseline_dirty_paths.contains(path))
        .collect::<Vec<_>>();
    let owned_paths = lease_owned_paths(db, session_id);
    let mut mismatches = Vec::new();
    let out_of_scope = derived
        .iter()
        .filter(|path| {
            !owned_paths.is_empty() && !crate::policy::owned_paths_cover_path(&owned_paths, path)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !out_of_scope.is_empty() {
        mismatches.push(format!(
            "wrote outside its owned-path lease ({}): {}",
            owned_paths.join(", "),
            out_of_scope.join(", ")
        ));
    }
    let claims_changes = !result.files_changed.is_empty();
    // A handoff makes the same claim a `completed` result does, and its change
    // set is what decides whether a completion gate opens over it, so an
    // unsupported claim has to be recorded here too.
    if matches!(
        result.status,
        WorkerResultStatus::Completed | WorkerResultStatus::NeedsDelegation
    ) && claims_changes
        && evidence.is_empty()
    {
        mismatches.push(format!(
            "reported {} changed file(s) but {} shows no commit past {} and a clean tree",
            result.files_changed.len(),
            recorded.worktree_path,
            recorded
                .base_commit
                .as_deref()
                .unwrap_or("the recorded base revision")
        ));
    }
    let unsupported = result
        .files_changed
        .iter()
        .filter(|path| !derived.iter().any(|actual| actual == *path))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() && !evidence.is_empty() {
        mismatches.push(format!(
            "claimed changes the repository does not show: {}",
            unsupported.join(", ")
        ));
    }
    let undeclared = derived
        .iter()
        .filter(|path| !result.files_changed.iter().any(|claimed| claimed == *path))
        .cloned()
        .collect::<Vec<_>>();
    if !undeclared.is_empty() {
        mismatches.push(format!(
            "changed files it did not report: {}",
            undeclared.join(", ")
        ));
    }

    let mut reconciled = result.clone();
    // The completion plan is built from `filesChanged`, so it must be the
    // repository's list, not the worker's. A worker cannot suppress a required
    // build or test by omitting a path — nor invent one by naming a file it never
    // touched, which is why the empty list replaces a claim too. Skipping the
    // assignment when Git found nothing left the worker's prose standing as the
    // change set, and a change set is what opens a gate.
    reconciled.files_changed = derived.clone();
    // Empty or out-of-scope evidence behind a `completed` claim is not a warning
    // to pass along; it is a failed claim.
    let fatal = mismatches
        .iter()
        .any(|mismatch| mismatch.starts_with("reported") || mismatch.starts_with("wrote outside"));
    if fatal && reconciled.status == WorkerResultStatus::Completed {
        reconciled.status = WorkerResultStatus::Blocked;
        reconciled.summary = format!(
            "Repository evidence does not support this result: {}. Original summary: {}",
            mismatches.join("; "),
            result.summary
        );
    }
    for mismatch in &mismatches {
        reconciled
            .risks
            .push(format!("Evidence mismatch — the worker {mismatch}"));
    }
    if fatal && reconciled.remaining_work.is_empty() {
        reconciled.remaining_work.push(
            "Re-run the work so the repository actually contains the change, or narrow the owned-path lease to what the task needs"
                .into(),
        );
    }
    ReconciledResult {
        result: reconciled,
        evidence: Some(evidence),
        binding: Some(recorded),
        mismatches,
    }
}

fn lease_owned_paths(db: &Connection, session_id: &str) -> Vec<String> {
    db.query_row(
        "SELECT owned_paths FROM worker_leases WHERE session_id=?1",
        params![session_id],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
    .unwrap_or_default()
}

/// Every child of this parent whose changes exist only in its own worktree.
pub fn pending_for_parent(
    db: &Connection,
    parent_session_id: &str,
) -> Result<Vec<WorkerRepositoryBinding>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "{SELECT} WHERE parent_session_id=?1 AND state IN (?2,?3) ORDER BY created_at,session_id"
    ))?;
    let rows = statement.query_map(
        params![parent_session_id, STATE_PENDING, STATE_SETTLING],
        map_binding,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// A validated adoption or discard, with the activity flags read while the
/// database lock was held. Split out so the caller can run Git — `git merge`,
/// `git worktree remove` — with no lock held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionPlan {
    pub binding: WorkerRepositoryBinding,
    pub task_active: bool,
    pub worker_active: bool,
    /// True when the state was already terminal: nothing to integrate, but a
    /// leaked worktree from an interrupted earlier attempt is still released.
    pub already_settled: bool,
    /// The completion attempt bound to this worker, and the repository stamp it
    /// was verified at. Adoption compares the tree it is about to merge with this;
    /// anything else is not the verified tree and must not inherit its proof.
    pub verified_attempt_id: Option<String>,
    pub verified_stamp: Option<(String, String)>,
}

/// The result of integrating a worker's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionOutcome {
    pub detail: String,
    /// False when the adopted tree is provably not the verified one: the worker
    /// checkout changed after verification, or the task branch advanced so the
    /// merge produced a combined tree nobody verified. The completion gate is
    /// re-opened rather than allowed to vouch for a tree it never saw.
    pub verified_tree_preserved: bool,
}

/// Validate an adoption and capture the activity flags it needs. Database phase.
pub fn plan_adoption(db: &Connection, session_id: &str) -> Result<AdoptionPlan, BridgeError> {
    let binding_row = binding(db, session_id)?.ok_or_else(|| {
        BridgeError::Invalid(format!("worker {session_id} has no repository binding"))
    })?;
    if binding_row.state == STATE_SETTLING {
        return Err(BridgeError::Invalid(format!(
            "another adopt or discard is already settling worker {session_id}; wait for it to finish"
        )));
    }
    let already_settled = binding_row.state == STATE_ADOPTED;
    if !already_settled && binding_row.state != STATE_PENDING {
        return Err(BridgeError::Invalid(format!(
            "worker {session_id} has nothing to adopt (state {})",
            binding_row.state
        )));
    }
    if !already_settled {
        claim_for_settling(db, session_id, "adopt")?;
    }
    let (verified_attempt_id, verified_stamp) = verified_attempt(db, session_id)?;
    Ok(AdoptionPlan {
        task_active: session_is_active(db, &binding_row.parent_session_id)?,
        worker_active: worker_is_reusable(db, &binding_row.session_id)?,
        already_settled,
        verified_attempt_id,
        verified_stamp,
        binding: binding_row,
    })
}

/// Take exclusive ownership of an adopt/discard for this binding. The compare-and-set
/// runs while the caller holds the database lock; Git then runs without it, so this
/// claim is the only thing preventing two concurrent decisions from interleaving.
fn claim_for_settling(
    db: &Connection,
    session_id: &str,
    operation: &str,
) -> Result<(), BridgeError> {
    let claimed = db.execute(
        "UPDATE worker_worktree_adoptions SET state=?2,detail=?3,updated_at=?4
         WHERE session_id=?1 AND state=?5",
        params![
            session_id,
            STATE_SETTLING,
            format!("{operation} in progress"),
            Utc::now().to_rfc3339(),
            STATE_PENDING,
        ],
    )?;
    if claimed != 1 {
        return Err(BridgeError::Invalid(format!(
            "another adopt or discard is already settling worker {session_id}; wait for it to finish"
        )));
    }
    Ok(())
}

/// Release a claim without settling it, so a failed attempt can be retried.
pub fn release_claim(db: &Connection, session_id: &str, reason: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worker_worktree_adoptions SET state=?2,detail=?3,updated_at=?4
         WHERE session_id=?1 AND state=?5",
        params![
            session_id,
            STATE_PENDING,
            reason,
            Utc::now().to_rfc3339(),
            STATE_SETTLING,
        ],
    )?;
    Ok(())
}

/// The live completion attempt for this worker's checkout, with the stamp it was
/// evaluated at.
fn verified_attempt(
    db: &Connection,
    session_id: &str,
) -> Result<(Option<String>, Option<(String, String)>), BridgeError> {
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT id,repository_head,dirty_digest FROM eval_attempts
             WHERE worker_session_id=?1 AND status NOT IN ('superseded')
             ORDER BY started_at DESC,rowid DESC LIMIT 1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some((id, head, dirty)) => (Some(id), Some((head, dirty))),
        None => (None, None),
    })
}

/// Validate a discard the same way. Database phase.
pub fn plan_discard(db: &Connection, session_id: &str) -> Result<AdoptionPlan, BridgeError> {
    let binding_row = binding(db, session_id)?.ok_or_else(|| {
        BridgeError::Invalid(format!("worker {session_id} has no repository binding"))
    })?;
    if binding_row.state == STATE_SETTLING {
        return Err(BridgeError::Invalid(format!(
            "another adopt or discard is already settling worker {session_id}; wait for it to finish"
        )));
    }
    let already_settled = binding_row.state == STATE_DISCARDED;
    if !already_settled && !binding_row.is_isolated() {
        return Err(BridgeError::Invalid(format!(
            "worker {session_id} wrote in place; there is no isolated output to discard"
        )));
    }
    // Output the user already adopted has been merged; calling it discarded would
    // record a decision that never happened and mislabel merged work.
    if binding_row.state == STATE_ADOPTED {
        return Err(BridgeError::Invalid(format!(
            "worker {session_id} was already adopted; its changes are merged and cannot be discarded"
        )));
    }
    if !already_settled {
        claim_for_settling(db, session_id, "discard")?;
    }
    Ok(AdoptionPlan {
        task_active: session_is_active(db, &binding_row.parent_session_id)?,
        worker_active: worker_is_reusable(db, &binding_row.session_id)?,
        already_settled,
        verified_attempt_id: None,
        verified_stamp: None,
        binding: binding_row,
    })
}

/// Merge a planned worker worktree into the task checkout. **Git only** — safe to
/// call with no database lock held. Integration refuses a dirty or active task
/// worktree and a non-fast-forward, so a failure leaves the work pending.
pub fn integrate(plan: &AdoptionPlan) -> Result<AdoptionOutcome, BridgeError> {
    if plan.already_settled {
        return Ok(AdoptionOutcome {
            detail: "already adopted".into(),
            verified_tree_preserved: true,
        });
    }
    let task = Path::new(&plan.binding.task_worktree_path);
    let worker = Path::new(&plan.binding.worktree_path);
    let mut divergence = Vec::new();
    // Does the checkout still hold exactly what was verified? The attempt stamped
    // this path's HEAD and dirty digest; anything else has been touched since.
    if let Some((head, dirty)) = &plan.verified_stamp {
        let current = store::repository_state_for_path(worker);
        let current_head = current.get("head").and_then(serde_json::Value::as_str);
        let current_dirty = current.get("dirtyHash").and_then(serde_json::Value::as_str);
        if current_head != Some(head.as_str()) || current_dirty != Some(dirty.as_str()) {
            divergence.push(format!(
                "the worker checkout changed after verification (verified {head}/{dirty}, found {}/{})",
                current_head.unwrap_or("unavailable"),
                current_dirty.unwrap_or("unavailable")
            ));
        }
    }
    // A three-way merge produces a tree that is neither the verified worker tree
    // nor the previously verified task tree.
    let fast_forward = git::integration_is_fast_forward(task, worker).unwrap_or(false);
    if !fast_forward {
        divergence.push(
            "the task branch advanced after verification, so integration merges into a combined tree that was never verified"
                .into(),
        );
    }
    // Capture anything the worker left uncommitted onto its own branch: a coding
    // harness usually does not commit, and integration requires a clean worker
    // tree. Without this the normal case would never be adoptable.
    let captured = git::commit_worker_worktree(
        worker,
        &format!(
            "bridge: capture worker {} output for adoption",
            plan.binding.session_id
        ),
    )?;
    let result = git::integrate_worker_changes(task, worker, plan.task_active)?;
    let mut detail = match captured {
        Some(commit) => format!("{result:?} (captured uncommitted work as {commit})"),
        None => format!("{result:?}"),
    };
    if !divergence.is_empty() {
        detail.push_str(&format!(
            "; re-verification required because {}",
            divergence.join(" and ")
        ));
    }
    Ok(AdoptionOutcome {
        detail,
        verified_tree_preserved: divergence.is_empty(),
    })
}

/// Record a terminal adoption/discard state and release the worktree. Database
/// phase; the worktree is removed only once the state is durably terminal, so a
/// crash between the two leaves recoverable work rather than a lost merge.
pub fn settle_plan(
    db: &Connection,
    plan: &AdoptionPlan,
    state: &str,
    detail: &str,
) -> Result<WorkerRepositoryBinding, BridgeError> {
    if !plan.already_settled {
        settle(db, &plan.binding.session_id, state, detail)?;
    }
    release_worktree(db, &plan.binding, plan.worker_active)?;
    binding(db, &plan.binding.session_id)?
        .ok_or_else(|| BridgeError::Invalid("settled worker binding disappeared".into()))
}

/// 10: a warm worker is idle, not finished — it can be resumed into this exact
/// checkout. Removing it would resume the worker into a directory that no longer
/// exists, so the worktree is retained until the worker is genuinely terminal;
/// [`recover`] and [`release_terminal_worktrees`] collect it afterwards.
fn worker_is_reusable(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    Ok(db
        .query_row(
            "SELECT s.status IN ('starting','working','resuming','checkpointing','waiting','warm','restored')
                 OR COALESCE(r.lifecycle_state,'') IN ('starting','working','resuming','checkpointing','waiting','warm','restored')
             FROM sessions s LEFT JOIN worker_runtime r ON r.session_id=s.id WHERE s.id=?1",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(true))
}

/// Collect child worktrees whose binding is terminal and whose worker can no
/// longer be resumed. Runs on the maintenance pass, because a worker that was
/// warm at adoption time becomes collectable later.
pub fn release_terminal_worktrees(db: &Connection) -> Result<usize, BridgeError> {
    let terminal = {
        let mut statement = db.prepare(&format!("{SELECT} WHERE state IN (?1,?2,?3)"))?;
        let rows = statement.query_map(
            params![STATE_ADOPTED, STATE_DISCARDED, STATE_EMPTY],
            map_binding,
        )?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut released = 0;
    for row in terminal {
        if !row.is_isolated() || !Path::new(&row.worktree_path).exists() {
            continue;
        }
        if worker_is_reusable(db, &row.session_id)? {
            continue;
        }
        release_worktree(db, &row, false)?;
        if !Path::new(&row.worktree_path).exists() {
            released += 1;
        }
    }
    released += settle_spent_empty_workers(db)?;
    Ok(released)
}

/// A worker that *stopped* never settled, so its binding stays
/// `pending_adoption` and every collector skipped it — the shape that held a
/// 3.9 GB checkout on the machine this was written on. Being unsettled is not
/// the same as holding work: when the checkout is clean and nothing landed past
/// the commit it was cut from, there is nothing for anyone to adopt, and
/// pretending otherwise leaks a directory to protect an empty diff.
///
/// A worker that stopped with real changes is a different case entirely and is
/// left exactly where it is, for the user to adopt or discard.
fn settle_spent_empty_workers(db: &Connection) -> Result<usize, BridgeError> {
    let pending = {
        let mut statement = db.prepare(&format!("{SELECT} WHERE state=?1"))?;
        let rows = statement.query_map(params![STATE_PENDING], map_binding)?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut released = 0;
    for row in pending {
        let worktree = Path::new(&row.worktree_path);
        if !row.is_isolated() || !worktree.exists() {
            continue;
        }
        if worker_is_reusable(db, &row.session_id)? {
            continue;
        }
        if !live_borrowers(db, &row)?.is_empty() {
            continue;
        }
        let Some(base) = row.base_commit.as_deref() else {
            continue;
        };
        // Both questions must answer "nothing here", and an unanswerable one
        // counts as work.
        if git::worktree_is_dirty(worktree).unwrap_or(true) {
            continue;
        }
        if git::commits_ahead_of(worktree, base).unwrap_or(1) != 0 {
            continue;
        }
        settle(
            db,
            &row.session_id,
            STATE_EMPTY,
            "the worker stopped without changing anything in its worktree",
        )?;
        let settled = binding(db, &row.session_id)?.unwrap_or(row);
        release_worktree(db, &settled, false)?;
        if !Path::new(&settled.worktree_path).exists() {
            released += 1;
        }
    }
    Ok(released)
}

/// Single-connection convenience used by tests and by callers that already hold
/// the only connection. Production goes through the phased pair so Git never runs
/// under the shared database lock.
pub fn adopt(db: &Connection, session_id: &str) -> Result<WorkerRepositoryBinding, BridgeError> {
    let plan = plan_adoption(db, session_id)?;
    match integrate(&plan) {
        Ok(outcome) => {
            let binding = settle_plan(db, &plan, STATE_ADOPTED, &outcome.detail)?;
            if !outcome.verified_tree_preserved {
                supersede_stale_proof(db, &plan, &outcome)?;
            }
            Ok(binding)
        }
        Err(error) => {
            // Integration refused (dirty or active task worktree, conflict). The
            // decision is still the user's, so hand the claim back.
            let _ = release_claim(db, session_id, &error.to_string());
            Err(error)
        }
    }
}

/// The adopted tree is not the tree the gate verified, so its proof must not
/// carry over: the attempt is superseded and the parent goes back to verifying.
pub fn supersede_stale_proof(
    db: &Connection,
    plan: &AdoptionPlan,
    outcome: &AdoptionOutcome,
) -> Result<(), BridgeError> {
    let Some(attempt_id) = &plan.verified_attempt_id else {
        return Ok(());
    };
    db.execute(
        "UPDATE eval_attempts SET status='superseded',completed_at=?2 WHERE id=?1 AND status NOT IN ('superseded')",
        params![attempt_id, Utc::now().to_rfc3339()],
    )?;
    store::event(
        db,
        "completion",
        "completion.proof_superseded_by_adoption",
        &plan.binding.parent_session_id,
        &outcome.detail,
    )?;
    Ok(())
}

/// Throw a worker's isolated output away on purpose.
pub fn discard(
    db: &Connection,
    session_id: &str,
    reason: &str,
) -> Result<WorkerRepositoryBinding, BridgeError> {
    let plan = plan_discard(db, session_id)?;
    settle_plan(db, &plan, STATE_DISCARDED, reason)
}

fn settle(db: &Connection, session_id: &str, state: &str, detail: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE worker_worktree_adoptions SET state=?2,detail=?3,updated_at=?4 WHERE session_id=?1",
        params![session_id, state, detail, Utc::now().to_rfc3339()],
    )?;
    store::event(
        db,
        "worktree",
        &format!("worker.worktree_{state}"),
        session_id,
        detail,
    )?;
    Ok(())
}

/// Every other live session running inside this checkout.
///
/// A verifier is bound to the implementation worker's worktree at launch, in its
/// own `worker_runtime` row — a different session, with no binding of its own on
/// that path. `worker_is_reusable` asks only about the worker that *wrote* there,
/// so adoption (and the terminal-worktree maintenance pass) could remove the
/// directory a reserved or running verifier was about to read.
fn live_borrowers(
    db: &Connection,
    binding_row: &WorkerRepositoryBinding,
) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT r.session_id FROM worker_runtime r
         LEFT JOIN sessions s ON s.id=r.session_id
         WHERE r.worktree_path=?1 AND r.session_id<>?2
           AND (COALESCE(s.status,'') IN ('starting','working','resuming','checkpointing','waiting','warm','restored')
                OR r.lifecycle_state IN ('starting','working','resuming','checkpointing','waiting','warm','restored'))
         ORDER BY r.session_id",
    )?;
    let rows = statement.query_map(
        params![binding_row.worktree_path, binding_row.session_id],
        |row| row.get::<_, String>(0),
    )?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(BridgeError::from)
}

/// Remove a child worktree whose state is terminal. A dirty worktree is left in
/// place: `safe_remove_worker_worktree` refuses it, and silently discarding
/// uncommitted work would be worse than leaking a directory. So is a worktree
/// another live worker is running in — see [`live_borrowers`].
fn release_worktree(
    db: &Connection,
    binding_row: &WorkerRepositoryBinding,
    worker_active: bool,
) -> Result<(), BridgeError> {
    if !binding_row.is_isolated() {
        return Ok(());
    }
    let worker = Path::new(&binding_row.worktree_path);
    if !worker.exists() {
        return Ok(());
    }
    let borrowers = live_borrowers(db, binding_row)?;
    if !borrowers.is_empty() {
        let _ = store::event(
            db,
            "worktree",
            "worker.worktree_retained",
            &binding_row.session_id,
            &format!(
                "cannot remove worker worktree while {} is still running in it",
                borrowers.join(", ")
            ),
        );
        return Ok(());
    }
    match git::safe_remove_worker_worktree(
        Path::new(&binding_row.task_worktree_path),
        worker,
        worker_active,
    ) {
        Ok(()) => {
            db.execute(
                "UPDATE worker_worktree_adoptions SET updated_at=?2 WHERE session_id=?1",
                params![binding_row.session_id, Utc::now().to_rfc3339()],
            )?;
            let _ = crate::worktree_registry::mark_removed(db, worker, "released on settlement");
            Ok(())
        }
        Err(error) => {
            let _ = store::event(
                db,
                "worktree",
                "worker.worktree_retained",
                &binding_row.session_id,
                &error.to_string(),
            );
            Ok(())
        }
    }
}

fn session_is_active(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    Ok(db
        .query_row(
            "SELECT status IN ('starting','working','resuming','checkpointing') FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Reconcile adoption state after a restart. A pending row whose worktree no
/// longer exists on disk cannot be adopted; recording that explicitly keeps the
/// parent from blocking forever on work that is already gone.
pub fn recover(db: &Connection) -> Result<usize, BridgeError> {
    let pending = {
        let mut statement = db.prepare(&format!("{SELECT} WHERE state IN (?1,?2)"))?;
        let rows = statement.query_map(params![STATE_PENDING, STATE_SETTLING], map_binding)?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    // A crash between claiming and settling leaves `settling` behind, which would
    // block the parent and refuse every retry. Restart returns it to pending.
    let unstuck = db.execute(
        "UPDATE worker_worktree_adoptions SET state=?1,detail=?2,updated_at=?3 WHERE state=?4",
        params![
            STATE_PENDING,
            "an adopt or discard was interrupted by a restart; the decision is still open",
            Utc::now().to_rfc3339(),
            STATE_SETTLING,
        ],
    )?;
    let mut reconciled = unstuck;
    for row in pending {
        if Path::new(&row.worktree_path).exists() {
            continue;
        }
        settle(
            db,
            &row.session_id,
            STATE_DISCARDED,
            "the worker worktree no longer exists on disk; nothing could be adopted after restart",
        )?;
        reconciled += 1;
    }
    Ok(reconciled)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        db: Connection,
        task: std::path::PathBuf,
        workers: std::path::PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let task = dir.path().join("task");
        std::fs::create_dir(&task).unwrap();
        git_cmd(&task, &["init", "-q", "-b", "main"]);
        git_cmd(
            &task,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git_cmd(
            &task,
            &["config", "commit.gpgsign", "false"],
        );
        git_cmd(&task, &["config", "user.name", "Bridge Test"]);
        std::fs::write(task.join("base.txt"), "base\n").unwrap();
        git_cmd(&task, &["add", "."]);
        git_cmd(&task, &["commit", "-q", "-m", "base"]);
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![task.to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','main',?1,'ready','now')", params![task.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth) VALUES('parent','w','codex','Parent','ready','reported',0)", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','completed','reported','parent',1)", []).unwrap();
        let workers = dir.path().join("workers");
        Fixture {
            _dir: dir,
            db,
            task,
            workers,
        }
    }

    /// Always derive against the base the binding recorded at launch: without
    /// it, a committed change is invisible and looks like "no change at all".
    fn recorded_evidence(fixture: &Fixture, worker: &Path) -> git::RepositoryEvidence {
        let base = binding(&fixture.db, "child").unwrap().unwrap().base_commit;
        git::derive_repository_evidence(worker, base.as_deref()).unwrap()
    }

    fn isolated_worker(fixture: &Fixture) -> std::path::PathBuf {
        let worker = git::create_child_worktree(
            &fixture.task,
            &fixture.workers,
            "child",
            "bridge/worker-child",
            &["src/**".to_owned()],
            &[],
        )
        .unwrap();
        record_binding(
            &fixture.db,
            "child",
            "parent",
            "w",
            &worker.path.to_string_lossy(),
            &worker.branch,
            &fixture.task.to_string_lossy(),
            true,
        )
        .unwrap();
        worker.path
    }

    /// The shape that held 3.9 GB on the machine this was written on: the worker
    /// *stopped* rather than settling, so its binding stayed `pending_adoption`
    /// and every collector skipped it — while the checkout held nothing anyone
    /// could adopt.
    #[test]
    fn a_stopped_worker_whose_worktree_holds_nothing_settles_and_releases_it() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING,
        );

        let released = release_terminal_worktrees(&fixture.db).unwrap();
        assert_eq!(released, 1);
        assert!(!worker.exists(), "the checkout is reclaimed");
        let settled = binding(&fixture.db, "child").unwrap().unwrap();
        assert_eq!(settled.state, STATE_EMPTY);
        assert!(
            settled
                .detail
                .unwrap_or_default()
                .contains("without changing anything"),
            "the settlement says why it was safe",
        );
    }

    #[test]
    fn a_stopped_worker_with_real_changes_keeps_its_worktree() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.rs").as_path(), "fn main() {}\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker output"]);

        let released = release_terminal_worktrees(&fixture.db).unwrap();
        assert_eq!(released, 0);
        assert!(worker.is_dir(), "unadopted work is never collected");
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING,
            "it stays the user's decision",
        );
    }

    #[test]
    fn a_stopped_worker_with_uncommitted_changes_keeps_its_worktree() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::write(worker.join("scratch.txt"), "unsaved\n").unwrap();

        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 0);
        assert!(worker.is_dir());
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING,
        );
    }

    /// `record_binding` derives the base at launch, so a missing one means the
    /// derivation failed. There is then no way to prove the checkout is empty,
    /// and an unprovable claim must not authorize a deletion.
    #[test]
    fn a_stopped_worker_with_no_recorded_base_is_left_alone() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        fixture
            .db
            .execute(
                "UPDATE worker_worktree_adoptions SET base_commit=NULL WHERE session_id='child'",
                [],
            )
            .unwrap();
        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 0);
        assert!(worker.is_dir());
    }

    #[test]
    fn a_committed_isolated_change_is_pending_then_merges_into_the_task_worktree() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);

        let base = binding(&fixture.db, "child").unwrap().unwrap().base_commit;
        let evidence = git::derive_repository_evidence(&worker, base.as_deref()).unwrap();
        assert_eq!(evidence.commits.len(), 1);
        assert_eq!(evidence.committed_paths, vec!["src/feature.txt"]);
        assert!(!evidence.is_empty());

        let recorded = record_evidence(&fixture.db, "child", &evidence)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.state, STATE_PENDING);
        assert_eq!(recorded.changed_paths, vec!["src/feature.txt"]);
        assert!(recorded.diffstat.unwrap().contains("1 file(s) changed"));
        assert_eq!(pending_for_parent(&fixture.db, "parent").unwrap().len(), 1);

        let adopted = adopt(&fixture.db, "child").unwrap();
        assert_eq!(adopted.state, STATE_ADOPTED);
        assert_eq!(
            std::fs::read_to_string(fixture.task.join("src/feature.txt")).unwrap(),
            "worker\n"
        );
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
        // Cleanup happens only after the state is terminal.
        assert!(!worker.exists());
    }

    /// The normal case for a coding harness: it edits files and does not commit.
    /// That work must still be adoptable — integration requires a clean worker
    /// tree, so adoption captures the edits onto the worker's own branch first.
    #[test]
    fn uncommitted_worker_output_is_captured_and_adopted_rather_than_stranded() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "uncommitted\n").unwrap();

        let evidence = recorded_evidence(&fixture, &worker);
        assert!(evidence.commits.is_empty(), "the worker committed nothing");
        assert_eq!(evidence.dirty_paths, vec!["src/feature.txt"]);
        let recorded = record_evidence(&fixture.db, "child", &evidence)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.state, STATE_PENDING);
        assert!(recorded.dirty);

        let adopted = adopt(&fixture.db, "child").unwrap();
        assert_eq!(adopted.state, STATE_ADOPTED);
        assert!(adopted
            .detail
            .unwrap()
            .contains("captured uncommitted work as"));
        assert_eq!(
            std::fs::read_to_string(fixture.task.join("src/feature.txt")).unwrap(),
            "uncommitted\n"
        );
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
    }

    /// An in-place writer shares the checkout with the user and with siblings.
    /// Their pre-existing edits must not be charged to it as out-of-scope writes.
    #[test]
    fn an_in_place_writer_is_not_blamed_for_edits_that_predate_it() {
        let fixture = fixture();
        // The user has an uncommitted edit outside the worker's lease.
        std::fs::write(fixture.task.join("README.md"), "user notes\n").unwrap();
        lease(&fixture, "[\"src/**\"]");
        record_binding(
            &fixture.db,
            "child",
            "parent",
            "w",
            &fixture.task.to_string_lossy(),
            "main",
            &fixture.task.to_string_lossy(),
            false,
        )
        .unwrap();
        let baseline = binding(&fixture.db, "child").unwrap().unwrap();
        assert_eq!(baseline.baseline_dirty_paths, vec!["README.md"]);
        assert_eq!(baseline.base_branch.as_deref(), Some("main"));

        // The worker then does its own, in-scope work.
        std::fs::create_dir_all(fixture.task.join("src")).unwrap();
        std::fs::write(fixture.task.join("src/feature.rs"), "ok\n").unwrap();

        let reconciled =
            reconcile_result_with_repository(&fixture.db, "child", &completed(&["src/feature.rs"]));

        assert!(
            reconciled.mismatches.is_empty(),
            "the user's own edit must not be attributed to the worker: {:?}",
            reconciled.mismatches
        );
        assert_eq!(reconciled.result.status, WorkerResultStatus::Completed);
        assert_eq!(reconciled.result.files_changed, vec!["src/feature.rs"]);
        assert_eq!(
            binding(&fixture.db, "child")
                .unwrap()
                .unwrap()
                .changed_paths,
            vec!["src/feature.rs"]
        );
    }

    /// Relaunching a warm worker must not re-open a decision already made about
    /// its previous output.
    #[test]
    fn rebinding_a_worker_never_reopens_a_settled_adoption() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        discard(&fixture.db, "child", "user rejected the approach").unwrap();

        // The same worker is resumed for a follow-up.
        record_binding(
            &fixture.db,
            "child",
            "parent",
            "w",
            &worker.to_string_lossy(),
            "bridge/worker-child",
            &fixture.task.to_string_lossy(),
            true,
        )
        .unwrap();

        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_DISCARDED
        );
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
    }

    /// Record a live completion attempt bound to this worker at its current stamp,
    /// so adoption can tell whether the tree it merges is the verified one.
    fn verify_at_current_state(fixture: &Fixture, worker: &Path) -> String {
        let state = store::repository_state_for_path(worker);
        let attempt = uuid::Uuid::new_v4().to_string();
        fixture.db.execute("INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_committed,status,created_at,updated_at) VALUES('c','w','parent',1,'[\"ok\"]',0,'active','now','now')", []).unwrap();
        fixture.db.execute("INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES('p','c',1,'low','{}','now')", []).unwrap();
        fixture.db.execute(
            "INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,worker_session_id,started_at)
             VALUES(?1,'p','parent',?2,?3,?4,'verified','child','now')",
            params![
                attempt,
                state["head"].as_str().unwrap(),
                state["dirtyHash"].as_str().unwrap(),
                worker.to_string_lossy(),
            ],
        )
        .unwrap();
        attempt
    }

    fn attempt_status(fixture: &Fixture, attempt: &str) -> String {
        fixture
            .db
            .query_row(
                "SELECT status FROM eval_attempts WHERE id=?1",
                params![attempt],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// Adoption may only hand its proof to the tree that was actually verified.
    #[test]
    fn adopting_the_verified_tree_keeps_its_proof() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        let attempt = verify_at_current_state(&fixture, &worker);

        let plan = plan_adoption(&fixture.db, "child").unwrap();
        let outcome = integrate(&plan).unwrap();
        assert!(outcome.verified_tree_preserved, "{}", outcome.detail);
        settle_plan(&fixture.db, &plan, STATE_ADOPTED, &outcome.detail).unwrap();
        assert_eq!(attempt_status(&fixture, &attempt), "verified");
    }

    /// The worker checkout changed after verification: the merged tree is not the
    /// verified tree, so the proof is superseded instead of silently transferred.
    #[test]
    fn a_worker_checkout_touched_after_verification_supersedes_its_proof() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "verified\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        let attempt = verify_at_current_state(&fixture, &worker);
        // Something edits the checkout after the gate passed.
        std::fs::write(
            worker.join("src/feature.txt"),
            "changed after verification\n",
        )
        .unwrap();

        adopt(&fixture.db, "child").unwrap();

        assert_eq!(attempt_status(&fixture, &attempt), "superseded");
        let detail = binding(&fixture.db, "child")
            .unwrap()
            .unwrap()
            .detail
            .unwrap();
        assert!(detail.contains("changed after verification"), "{detail}");
    }

    /// The task branch advanced, so integration is a three-way merge producing a
    /// combined tree nobody verified.
    #[test]
    fn a_three_way_merge_supersedes_the_proof_it_was_never_given() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        let attempt = verify_at_current_state(&fixture, &worker);
        // The task branch moves on, disjointly, after verification.
        std::fs::create_dir_all(fixture.task.join("docs")).unwrap();
        std::fs::write(fixture.task.join("docs/notes.md"), "task side\n").unwrap();
        git_cmd(&fixture.task, &["add", "."]);
        git_cmd(&fixture.task, &["commit", "-q", "-m", "task edit"]);

        adopt(&fixture.db, "child").unwrap();

        assert_eq!(attempt_status(&fixture, &attempt), "superseded");
        let detail = binding(&fixture.db, "child")
            .unwrap()
            .unwrap()
            .detail
            .unwrap();
        assert!(detail.contains("task branch advanced"), "{detail}");
        // The merge still happened — the work is not lost, only re-verified.
        assert!(fixture.task.join("src/feature.txt").exists());
    }

    /// Git runs with the database lock released, so two decisions must not
    /// interleave into "merged, then recorded discarded".
    #[test]
    fn a_second_decision_cannot_interleave_with_one_in_flight() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();

        let plan = plan_adoption(&fixture.db, "child").unwrap();
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_SETTLING
        );
        // While that adoption is mid-Git, a concurrent discard must be refused.
        let refused = plan_discard(&fixture.db, "child").unwrap_err().to_string();
        assert!(refused.contains("already settling"), "{refused}");
        assert!(plan_adoption(&fixture.db, "child")
            .unwrap_err()
            .to_string()
            .contains("already settling"));
        // A claimed binding still blocks the parent.
        assert_eq!(pending_for_parent(&fixture.db, "parent").unwrap().len(), 1);

        let outcome = integrate(&plan).unwrap();
        settle_plan(&fixture.db, &plan, STATE_ADOPTED, &outcome.detail).unwrap();
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_ADOPTED
        );
        // And adopted output can never be relabelled as discarded.
        assert!(discard(&fixture.db, "child", "changed my mind")
            .unwrap_err()
            .to_string()
            .contains("already adopted"));
    }

    #[test]
    fn a_refused_integration_hands_the_decision_back_instead_of_wedging_it() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::write(worker.join("base.txt"), "worker version\n").unwrap();
        git_cmd(&worker, &["commit", "-q", "-am", "worker edit"]);
        std::fs::write(fixture.task.join("base.txt"), "task version\n").unwrap();
        git_cmd(&fixture.task, &["commit", "-q", "-am", "task edit"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();

        assert!(adopt(&fixture.db, "child").is_err());
        // Still the user's decision to make, and still retryable.
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING
        );
        assert!(discard(&fixture.db, "child", "conflicts with the task branch").is_ok());
    }

    /// A restart between claiming and settling must not wedge the decision.
    #[test]
    fn an_interrupted_decision_is_reopened_after_restart() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        let _claimed = plan_adoption(&fixture.db, "child").unwrap();

        assert!(recover(&fixture.db).unwrap() >= 1);
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING
        );
        assert!(adopt(&fixture.db, "child").is_ok());
    }

    /// A verifier runs in the implementation worker's checkout, in its own
    /// session, with no binding of its own on that path. Asking only whether the
    /// worker that *wrote* there is still reusable let adoption remove the
    /// directory the verifier was about to read.
    #[test]
    fn a_worktree_another_live_worker_runs_in_is_retained() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        // A verifier bound to the implementation worker's checkout, reserved and
        // about to start.
        fixture.db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('verifier','w','codex','Verification','starting','reported','parent',1)", []).unwrap();
        fixture.db.execute("INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,worktree_path,updated_at) VALUES('verifier','parent','starting','verification','key','pending',0,?1,'now')", params![worker.to_string_lossy()]).unwrap();

        adopt(&fixture.db, "child").unwrap();
        assert!(
            worker.exists(),
            "the verifier's checkout must survive the adoption of the work it is verifying"
        );
        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 0);
        assert!(fixture
            .db
            .query_row(
                "SELECT body FROM events WHERE kind='worker.worktree_retained' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            .contains("verifier"));

        // Once verification is terminal, the worktree is collected.
        fixture
            .db
            .execute(
                "UPDATE sessions SET status='stopped' WHERE id='verifier'",
                [],
            )
            .unwrap();
        fixture
            .db
            .execute(
                "UPDATE worker_runtime SET lifecycle_state='completed' WHERE session_id='verifier'",
                [],
            )
            .unwrap();
        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 1);
        assert!(!worker.exists());
    }

    /// A warm worker is idle, not finished: it can be resumed into this exact
    /// checkout, so settling must not delete it out from under a later resume.
    #[test]
    fn a_warm_workers_checkout_survives_adoption_until_it_is_terminal() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        record_evidence(&fixture.db, "child", &recorded_evidence(&fixture, &worker)).unwrap();
        fixture
            .db
            .execute("UPDATE sessions SET status='warm' WHERE id='child'", [])
            .unwrap();

        adopt(&fixture.db, "child").unwrap();
        assert!(
            worker.exists(),
            "a resumable worker must keep the checkout it would resume into"
        );
        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 0);

        // Once it can no longer be resumed, the worktree is collected.
        fixture
            .db
            .execute("UPDATE sessions SET status='stopped' WHERE id='child'", [])
            .unwrap();
        assert_eq!(release_terminal_worktrees(&fixture.db).unwrap(), 1);
        assert!(!worker.exists());
    }

    #[test]
    fn an_isolated_worker_that_changed_nothing_needs_no_adoption() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        let evidence = recorded_evidence(&fixture, &worker);
        assert!(evidence.is_empty());
        let recorded = record_evidence(&fixture.db, "child", &evidence)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.state, STATE_EMPTY);
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn discarding_is_explicit_and_releases_the_worktree() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        let evidence = recorded_evidence(&fixture, &worker);
        assert_eq!(
            record_evidence(&fixture.db, "child", &evidence)
                .unwrap()
                .unwrap()
                .state,
            STATE_PENDING
        );

        let discarded = discard(&fixture.db, "child", "user rejected the approach").unwrap();
        assert_eq!(discarded.state, STATE_DISCARDED);
        assert!(!fixture.task.join("src/feature.txt").exists());
        assert!(!worker.exists());
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
        // Idempotent: a repeated discard is not an error.
        assert_eq!(
            discard(&fixture.db, "child", "again").unwrap().state,
            STATE_DISCARDED
        );
    }

    #[test]
    fn a_shared_writer_is_recorded_in_place_and_never_awaits_adoption() {
        let fixture = fixture();
        record_binding(
            &fixture.db,
            "child",
            "parent",
            "w",
            &fixture.task.to_string_lossy(),
            "main",
            &fixture.task.to_string_lossy(),
            false,
        )
        .unwrap();
        let row = binding(&fixture.db, "child").unwrap().unwrap();
        assert_eq!(row.state, STATE_IN_PLACE);
        assert!(!row.is_isolated());
        assert!(row.base_commit.is_some());
        std::fs::write(fixture.task.join("base.txt"), "changed\n").unwrap();
        let evidence =
            git::derive_repository_evidence(&fixture.task, row.base_commit.as_deref()).unwrap();
        assert_eq!(evidence.dirty_paths, vec!["base.txt"]);
        let recorded = record_evidence(&fixture.db, "child", &evidence)
            .unwrap()
            .unwrap();
        assert_eq!(recorded.state, STATE_IN_PLACE);
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
        assert!(discard(&fixture.db, "child", "n/a").is_err());
    }

    #[test]
    fn a_pending_worktree_missing_after_restart_is_reconciled_instead_of_blocking_forever() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.txt"), "worker\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "worker change"]);
        let evidence = recorded_evidence(&fixture, &worker);
        record_evidence(&fixture.db, "child", &evidence).unwrap();
        assert_eq!(recover(&fixture.db).unwrap(), 0);

        std::fs::remove_dir_all(&worker).unwrap();
        assert_eq!(recover(&fixture.db).unwrap(), 1);
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_DISCARDED
        );
        assert!(pending_for_parent(&fixture.db, "parent")
            .unwrap()
            .is_empty());
    }

    fn completed(files_changed: &[&str]) -> WorkerResult {
        WorkerResult {
            schema_version: crate::delegation::SCHEMA_VERSION,
            status: WorkerResultStatus::Completed,
            summary: "Rendered Mermaid, math, and sandboxed HTML inline".into(),
            files_changed: files_changed
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec![],
            suggested_next_action: crate::delegation::SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        }
    }

    fn lease(fixture: &Fixture, owned_paths: &str) {
        fixture
            .db
            .execute(
                "INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,created_at,updated_at)
                 VALUES('child','w','implementation','strong','implementation',?1,'isolated','active','now','now')",
                params![owned_paths],
            )
            .unwrap();
    }

    /// The incident in one test: a worker reported five changed files and a
    /// `completed` status against a clean repository. Schema validation passed it
    /// through, and the completion planner then chose its checks from that prose.
    #[test]
    fn a_completed_claim_with_no_repository_change_is_downgraded_to_blocked() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        lease(&fixture, "[\"src/**\"]");
        assert!(worker.exists());

        let claim = completed(&[
            "src/components/Markdown.tsx",
            "src/components/Markdown.test.tsx",
            "src/index.css",
            "package.json",
            "bun.lock",
        ]);
        let reconciled = reconcile_result_with_repository(&fixture.db, "child", &claim);

        assert_eq!(reconciled.result.status, WorkerResultStatus::Blocked);
        assert!(
            reconciled
                .result
                .summary
                .contains("does not support this result"),
            "{}",
            reconciled.result.summary
        );
        assert!(reconciled
            .mismatches
            .iter()
            .any(|mismatch| mismatch.contains("shows no commit past")));
        assert!(!reconciled.result.remaining_work.is_empty());
        // Nothing to adopt, because nothing was written.
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_EMPTY
        );
    }

    /// The completion planner picks required checks from `filesChanged`, so a
    /// worker must not be able to steer it by omitting paths.
    #[test]
    fn derived_paths_replace_the_workers_claim_so_checks_cannot_be_suppressed() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        lease(&fixture, "[\"src/**\"]");
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/renderer.rs"), "fn main() {}\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "rust change"]);

        // The worker reports only a doc file, which would plan no Rust checks.
        let reconciled =
            reconcile_result_with_repository(&fixture.db, "child", &completed(&["docs/notes.md"]));

        assert_eq!(reconciled.result.files_changed, vec!["src/renderer.rs"]);
        assert!(reconciled
            .mismatches
            .iter()
            .any(|mismatch| mismatch.contains("claimed changes the repository does not show")));
        assert!(reconciled
            .mismatches
            .iter()
            .any(|mismatch| mismatch.contains("changed files it did not report")));
        // Undeclared and unsupported paths are reporting problems, not lies about
        // whether work happened, so the status survives.
        assert_eq!(reconciled.result.status, WorkerResultStatus::Completed);
        let evidence = reconciled.evidence.unwrap();
        assert_eq!(evidence.commits.len(), 1);
        assert_eq!(evidence.branch.as_deref(), Some("bridge/worker-child"));
        assert!(evidence.diffstat().contains("1 file(s) changed"));
    }

    /// A handoff makes the same claim a `completed` result does, and its change
    /// set decides whether a completion gate opens over it. The derived list used
    /// to replace the claim only when Git found something, so a fabricated
    /// `filesChanged` survived a clean tree and could open a gate over nothing.
    #[test]
    fn a_fabricated_change_set_cannot_open_a_gate() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        lease(&fixture, "[\"src/**\"]");
        assert!(worker.exists());

        let mut handoff = completed(&["src/router.rs", "src/router.test.ts"]);
        handoff.status = WorkerResultStatus::NeedsDelegation;
        handoff.suggested_task = Some("verify the router change".into());
        handoff.suggested_role = Some(crate::delegation::WorkerRole::Verification);
        let reconciled = reconcile_result_with_repository(&fixture.db, "child", &handoff);

        assert!(
            reconciled.result.files_changed.is_empty(),
            "the repository shows nothing, so the change set is empty: {:?}",
            reconciled.result.files_changed
        );
        assert!(reconciled
            .mismatches
            .iter()
            .any(|mismatch| mismatch.contains("shows no commit past")));
        // The status survives: downgrading a handoff would discard the follow-up
        // it asked for, and with an empty change set it can no longer open a gate.
        assert_eq!(
            reconciled.result.status,
            WorkerResultStatus::NeedsDelegation
        );
        assert_eq!(
            reconciled.result.suggested_task.as_deref(),
            Some("verify the router change")
        );
    }

    #[test]
    fn writing_outside_the_owned_path_lease_is_a_fatal_evidence_mismatch() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        lease(&fixture, "[\"src/**\"]");
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/allowed.rs"), "ok\n").unwrap();
        std::fs::write(worker.join("secrets.env"), "TOKEN=1\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "in and out of scope"]);

        let reconciled = reconcile_result_with_repository(
            &fixture.db,
            "child",
            &completed(&["src/allowed.rs", "secrets.env"]),
        );

        assert_eq!(reconciled.result.status, WorkerResultStatus::Blocked);
        assert!(reconciled.mismatches.iter().any(|mismatch| mismatch
            .contains("wrote outside its owned-path lease")
            && mismatch.contains("secrets.env")));
    }

    #[test]
    fn a_claim_the_repository_backs_up_passes_through_unchanged() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        lease(&fixture, "[\"src/**\"]");
        std::fs::create_dir_all(worker.join("src")).unwrap();
        std::fs::write(worker.join("src/feature.rs"), "ok\n").unwrap();
        git_cmd(&worker, &["add", "."]);
        git_cmd(&worker, &["commit", "-q", "-m", "feature"]);

        let reconciled =
            reconcile_result_with_repository(&fixture.db, "child", &completed(&["src/feature.rs"]));

        assert!(reconciled.mismatches.is_empty());
        assert_eq!(reconciled.result, completed(&["src/feature.rs"]));
        assert_eq!(
            reconciled.binding.unwrap().state,
            STATE_PENDING,
            "verified isolated work still needs adoption"
        );
    }

    /// A read-only worker has no binding; its guard is the read-only sandbox, and
    /// reconciliation must not invent a mismatch for it.
    #[test]
    fn a_worker_with_no_binding_is_left_exactly_as_reported() {
        let fixture = fixture();
        let claim = completed(&["src/notes.md"]);
        let reconciled = reconcile_result_with_repository(&fixture.db, "child", &claim);
        assert_eq!(reconciled.result, claim);
        assert!(reconciled.evidence.is_none());
        assert!(reconciled.mismatches.is_empty());
    }

    #[test]
    fn adoption_that_cannot_merge_leaves_the_work_pending() {
        let fixture = fixture();
        let worker = isolated_worker(&fixture);
        std::fs::write(worker.join("base.txt"), "worker version\n").unwrap();
        git_cmd(&worker, &["commit", "-q", "-am", "worker edit"]);
        std::fs::write(fixture.task.join("base.txt"), "task version\n").unwrap();
        git_cmd(&fixture.task, &["commit", "-q", "-am", "task edit"]);
        let evidence = recorded_evidence(&fixture, &worker);
        record_evidence(&fixture.db, "child", &evidence).unwrap();

        assert!(adopt(&fixture.db, "child").is_err());
        assert_eq!(
            binding(&fixture.db, "child").unwrap().unwrap().state,
            STATE_PENDING
        );
        assert!(worker.exists());
        assert_eq!(
            std::fs::read_to_string(fixture.task.join("base.txt")).unwrap(),
            "task version\n"
        );
    }
}
