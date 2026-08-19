//! What happens to a task between briefings.
//!
//! Two rules run through all of it.
//!
//! **Absence only means something when the source was read.** A task that stops appearing
//! might be finished, or its connector might have been down — and those must not look the
//! same. So a miss counts only when *that task's own* source succeeded in that run, which
//! is why the count is stored rather than derived from a history of briefs.
//!
//! **A human's decision outranks the model's.** Done, dismissed, snoozed and pinned are
//! the user talking. A reconciliation that reset them because the brief changed would make
//! those buttons decorative, so each survives every run that does not specifically undo it.

use bridge_protocol::messages as wire;

/// How many consecutive successful misses make a task stale.
///
/// Two, not one: one absence from a brief is ordinary — a model ranks twelve things and
/// this was thirteenth. Two says the source was read twice and did not mention it.
pub const MISSES_BEFORE_STALE: i64 = 2;

/// What a run learned about one source, from that run's coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOutcome {
    /// Read successfully. The only outcome that can age a task.
    Read,
    /// Tried and failed, needed auth, was never offered, or was never reached because the
    /// run stopped. None of these say anything about whether a task still exists.
    NotRead,
}

impl SourceOutcome {
    /// Read a coverage status. Only `Succeeded` counts as having read the source: a
    /// consulted-but-failed call produced no answer, so it is not evidence of absence.
    pub fn from_coverage(status: wire::WorkSourceStatus) -> Self {
        match status {
            wire::WorkSourceStatus::Succeeded => Self::Read,
            wire::WorkSourceStatus::Consulted
            | wire::WorkSourceStatus::Failed
            | wire::WorkSourceStatus::AuthRequired
            | wire::WorkSourceStatus::Eligible
            | wire::WorkSourceStatus::Ineligible => Self::NotRead,
        }
    }

    pub fn read_successfully(self) -> bool {
        matches!(self, Self::Read)
    }
}

/// A task's state, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Active,
    Snoozed,
    Done,
    Dismissed,
    Stale,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Snoozed => "snoozed",
            Self::Done => "done",
            Self::Dismissed => "dismissed",
            Self::Stale => "stale",
        }
    }

    /// A state this build does not recognise reads as `dismissed`.
    ///
    /// Fail-closed for a *board*: showing a row whose meaning is unknown is worse than
    /// hiding one, because a user acting on it would be acting on a guess.
    pub fn parse(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "snoozed" => Self::Snoozed,
            "done" => Self::Done,
            "stale" => Self::Stale,
            _ => Self::Dismissed,
        }
    }

    /// Does a reconciliation still consider this task?
    ///
    /// A snoozed task does, so an expiring snooze reveals current information rather than
    /// a snapshot from when it was set. A dismissed one does not: the user said no.
    pub fn reconciles(self) -> bool {
        !matches!(self, Self::Dismissed)
    }

    /// Is this task on the board a human reads?
    pub fn visible(self, pinned: bool) -> bool {
        match self {
            Self::Active => true,
            // A pinned task stays visible when it goes stale — pinning is how a user says
            // "keep this in front of me", and a stale pin is exactly when they meant it.
            Self::Stale => pinned,
            Self::Snoozed | Self::Done | Self::Dismissed => false,
        }
    }
}

/// What a human asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    Complete,
    Snooze,
    Dismiss,
    Restore,
    Pin,
    Unpin,
}

/// Why an action was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: &'static str,
    pub action: &'static str,
}

impl IllegalTransition {
    pub fn reason(&self) -> String {
        format!("a {} task cannot be {}", self.from, self.action)
    }
}

fn action_name(action: TaskAction) -> &'static str {
    match action {
        TaskAction::Complete => "completed",
        TaskAction::Snooze => "snoozed",
        TaskAction::Dismiss => "dismissed",
        TaskAction::Restore => "restored",
        TaskAction::Pin => "pinned",
        TaskAction::Unpin => "unpinned",
    }
}

/// A task as far as this module is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub state: TaskState,
    pub pinned: bool,
    pub miss_count: i64,
    pub ephemeral: bool,
    /// When the user resolved it, for the newer-evidence rule.
    pub resolved_at: Option<String>,
    /// The evidence currently backing the task.
    pub evidence_digest: Option<String>,
    /// The evidence the user saw when completing the task.
    pub resolved_evidence_digest: Option<String>,
}

impl TaskSnapshot {
    pub fn active() -> Self {
        Self {
            state: TaskState::Active,
            pinned: false,
            miss_count: 0,
            ephemeral: false,
            resolved_at: None,
            evidence_digest: None,
            resolved_evidence_digest: None,
        }
    }
}

/// Apply a human action.
///
/// Pinning is orthogonal: it never changes the state, and no state forbids it. That is the
/// point of a pin — it survives whatever the task is doing.
pub fn apply_action(
    task: &TaskSnapshot,
    action: TaskAction,
    at: &str,
) -> Result<TaskSnapshot, IllegalTransition> {
    let mut next = task.clone();
    match action {
        TaskAction::Pin => {
            next.pinned = true;
            return Ok(next);
        }
        TaskAction::Unpin => {
            next.pinned = false;
            return Ok(next);
        }
        _ => {}
    }

    let illegal = |from: TaskState| {
        Err(IllegalTransition {
            from: from.as_str(),
            action: action_name(action),
        })
    };

    match (task.state, action) {
        // Restore is only meaningful for something the user put away.
        (TaskState::Dismissed, TaskAction::Restore) | (TaskState::Snoozed, TaskAction::Restore) => {
            next.state = if task.miss_count >= MISSES_BEFORE_STALE {
                TaskState::Stale
            } else {
                TaskState::Active
            };
            next.resolved_at = None;
            next.resolved_evidence_digest = None;
        }
        (_, TaskAction::Restore) => return illegal(task.state),

        // Completing or dismissing is available from anywhere a task is still live.
        (TaskState::Active | TaskState::Snoozed | TaskState::Stale, TaskAction::Complete) => {
            next.state = TaskState::Done;
            next.resolved_at = Some(at.to_owned());
            next.resolved_evidence_digest = task.evidence_digest.clone();
        }
        (TaskState::Active | TaskState::Snoozed | TaskState::Stale, TaskAction::Dismiss) => {
            next.state = TaskState::Dismissed;
            next.resolved_at = Some(at.to_owned());
        }
        (TaskState::Active | TaskState::Stale, TaskAction::Snooze) => {
            next.state = TaskState::Snoozed;
            next.resolved_at = Some(at.to_owned());
        }
        // Re-snoozing, re-completing, or dismissing what is already dismissed are all
        // refused rather than silently accepted: a no-op that reports success invites a
        // caller to believe something happened.
        _ => return illegal(task.state),
    }
    Ok(next)
}

/// What a reconciliation does to one task that the new brief did **not** mention.
///
/// `outcome` is what happened to that task's own source in that run.
pub fn reconcile_absent(task: &TaskSnapshot, outcome: SourceOutcome) -> TaskSnapshot {
    let mut next = task.clone();
    if !task.state.reconciles() {
        // Dismissed stays dismissed. A later brief does not undo a user's no.
        return next;
    }
    if !outcome.read_successfully() {
        // The source was not read, so its silence is not evidence. Ageing here is the bug
        // this whole module is shaped around avoiding.
        return next;
    }
    // Once absence has reached its terminal meaning, further silent reads add no
    // information. Capping here also prevents historical rows being rewritten forever.
    if task.miss_count >= MISSES_BEFORE_STALE {
        return next;
    }
    next.miss_count = task.miss_count.saturating_add(1);
    if next.miss_count >= MISSES_BEFORE_STALE && matches!(task.state, TaskState::Active) {
        next.state = TaskState::Stale;
    }
    next
}

/// What a reconciliation does to one task the new brief **did** mention.
///
/// `evidence_digest` is the content Bridge saw behind it. Collection time cannot decide
/// whether a completed task reopens: rereading unchanged content would otherwise resurrect it.
pub fn reconcile_present(task: &TaskSnapshot, evidence_digest: Option<&str>) -> TaskSnapshot {
    let mut next = task.clone();
    if !task.state.reconciles() {
        return next;
    }
    // Reappearing clears the count however the task is doing: the source mentioned it, so
    // the run of silences is over.
    next.miss_count = 0;
    match task.state {
        TaskState::Stale => next.state = TaskState::Active,
        TaskState::Done => {
            // A completed task reopens only when its backing content changed. A missing
            // completion snapshot fails closed, preserving a decision made by an older build.
            if task.resolved_evidence_digest.as_deref().is_some_and(|resolved| {
                evidence_digest.is_some_and(|current| current != resolved)
            }) {
                next.state = TaskState::Active;
                next.resolved_at = None;
                next.resolved_evidence_digest = None;
            }
        }
        // A snooze is a decision about time, not about the brief, so it survives being
        // mentioned again.
        TaskState::Snoozed | TaskState::Active | TaskState::Dismissed => {}
    }
    next
}

fn parse_instant(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
}

/// Has a snooze run out?
pub fn snooze_expired(snoozed_until: Option<&str>, now: &str) -> bool {
    match (snoozed_until.and_then(parse_instant), parse_instant(now)) {
        (Some(until), Some(now)) => now >= until,
        // No deadline, or a deadline nobody can read, leaves the snooze in place. A snooze
        // that expired because a timestamp was unparseable would surprise the user who set
        // it.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOLVED: &str = "2026-08-19T12:00:00+00:00";
    const AFTER: &str = "2026-08-19T13:00:00+00:00";

    fn task(state: TaskState) -> TaskSnapshot {
        TaskSnapshot { state, ..TaskSnapshot::active() }
    }

    // -----------------------------------------------------------------------
    // Absence only means something when the source was read
    // -----------------------------------------------------------------------

    #[test]
    fn only_a_successful_read_of_a_tasks_own_source_ages_it() {
        let aged = reconcile_absent(&TaskSnapshot::active(), SourceOutcome::Read);
        assert_eq!(aged.miss_count, 1);
        let untouched = reconcile_absent(&TaskSnapshot::active(), SourceOutcome::NotRead);
        assert_eq!(untouched.miss_count, 0);
    }

    #[test]
    fn no_unsuccessful_coverage_state_ages_a_task() {
        // Each of these is a different reason a source said nothing, and none of them is
        // evidence that a task is gone.
        for status in [
            wire::WorkSourceStatus::Failed,
            wire::WorkSourceStatus::AuthRequired,
            wire::WorkSourceStatus::Consulted,
            wire::WorkSourceStatus::Eligible,
            wire::WorkSourceStatus::Ineligible,
        ] {
            let outcome = SourceOutcome::from_coverage(status);
            assert!(!outcome.read_successfully(), "{status:?} is not a successful read");
            let after = reconcile_absent(&TaskSnapshot::active(), outcome);
            assert_eq!(after.miss_count, 0, "{status:?} must not age a task");
            assert_eq!(after.state, TaskState::Active);
        }
        assert!(SourceOutcome::from_coverage(wire::WorkSourceStatus::Succeeded).read_successfully());
    }

    #[test]
    fn a_consulted_but_failed_call_is_not_a_read() {
        // The subtle one: the model called the tool, so coverage advanced past Eligible —
        // but no answer came back, so the source said nothing.
        assert!(!SourceOutcome::from_coverage(wire::WorkSourceStatus::Consulted).read_successfully());
        assert!(!SourceOutcome::from_coverage(wire::WorkSourceStatus::Failed).read_successfully());
    }

    #[test]
    fn two_successful_misses_make_a_task_stale_and_one_does_not() {
        let once = reconcile_absent(&TaskSnapshot::active(), SourceOutcome::Read);
        assert_eq!(once.state, TaskState::Active, "one absence is ordinary");
        assert_eq!(once.miss_count, 1);
        let twice = reconcile_absent(&once, SourceOutcome::Read);
        assert_eq!(twice.state, TaskState::Stale);
        assert_eq!(twice.miss_count, MISSES_BEFORE_STALE);
    }

    #[test]
    fn a_failed_source_between_two_reads_does_not_shorten_the_path_to_stale() {
        // Read, fail, read: two successful misses, so stale — the failure neither ages nor
        // resets, it simply is not evidence either way.
        let after_read = reconcile_absent(&TaskSnapshot::active(), SourceOutcome::Read);
        let after_failure = reconcile_absent(&after_read, SourceOutcome::NotRead);
        assert_eq!(after_failure.miss_count, 1);
        let after_second_read = reconcile_absent(&after_failure, SourceOutcome::Read);
        assert_eq!(after_second_read.state, TaskState::Stale);
    }

    #[test]
    fn a_reappearing_task_resets_its_miss_count() {
        let aged = reconcile_absent(&TaskSnapshot::active(), SourceOutcome::Read);
        let back = reconcile_present(&aged, Some("digest-1"));
        assert_eq!(back.miss_count, 0);
        assert_eq!(back.state, TaskState::Active);
    }

    #[test]
    fn a_stale_task_that_reappears_becomes_active_again() {
        let stale = TaskSnapshot { state: TaskState::Stale, miss_count: 2, ..TaskSnapshot::active() };
        let back = reconcile_present(&stale, Some("digest-1"));
        assert_eq!(back.state, TaskState::Active);
        assert_eq!(back.miss_count, 0);
    }

    // -----------------------------------------------------------------------
    // A human's decision outranks the model's
    // -----------------------------------------------------------------------

    #[test]
    fn a_snoozed_task_keeps_reconciling() {
        // So an expiring snooze reveals current information rather than a snapshot from
        // when it was set.
        let snoozed = task(TaskState::Snoozed);
        assert!(snoozed.state.reconciles());
        let aged = reconcile_absent(&snoozed, SourceOutcome::Read);
        assert_eq!(aged.miss_count, 1, "it is still being tracked");
        assert_eq!(aged.state, TaskState::Snoozed, "but the snooze is not overridden");
        let mentioned = reconcile_present(&snoozed, Some("digest-1"));
        assert_eq!(mentioned.state, TaskState::Snoozed, "a snooze is about time, not the brief");
    }

    #[test]
    fn a_snoozed_task_does_not_go_stale_out_from_under_its_snooze() {
        let snoozed = TaskSnapshot { state: TaskState::Snoozed, miss_count: 1, ..TaskSnapshot::active() };
        let aged = reconcile_absent(&snoozed, SourceOutcome::Read);
        assert_eq!(aged.miss_count, 2);
        assert_eq!(aged.state, TaskState::Snoozed, "only an active task becomes stale");
    }

    #[test]
    fn a_done_task_reopens_only_on_changed_evidence() {
        let done = TaskSnapshot {
            state: TaskState::Done,
            resolved_at: Some(RESOLVED.into()),
            evidence_digest: Some("digest-1".into()),
            resolved_evidence_digest: Some("digest-1".into()),
            ..TaskSnapshot::active()
        };
        let reopened = reconcile_present(&done, Some("digest-2"));
        assert_eq!(reopened.state, TaskState::Active);
        assert!(reopened.resolved_at.is_none());
        assert!(reopened.resolved_evidence_digest.is_none());
        // Rereading the same content later is not news.
        for unchanged_evidence in [Some("digest-1"), None] {
            let untouched = reconcile_present(&done, unchanged_evidence);
            assert_eq!(untouched.state, TaskState::Done, "{unchanged_evidence:?} must not reopen it");
            assert_eq!(untouched.resolved_at.as_deref(), Some(RESOLVED));
        }
    }

    #[test]
    fn a_missing_completion_digest_leaves_a_done_task_done() {
        // Older rows have no content snapshot. Undoing their decision is the wrong default.
        let done = TaskSnapshot {
            state: TaskState::Done,
            resolved_at: Some("whenever".into()),
            ..TaskSnapshot::active()
        };
        assert_eq!(reconcile_present(&done, Some("digest-2")).state, TaskState::Done);
    }

    #[test]
    fn a_dismissed_task_stays_suppressed_until_restore() {
        let dismissed = task(TaskState::Dismissed);
        assert!(!dismissed.state.reconciles());
        assert_eq!(reconcile_present(&dismissed, Some("digest-1")).state, TaskState::Dismissed);
        assert_eq!(reconcile_absent(&dismissed, SourceOutcome::Read).state, TaskState::Dismissed);
        assert_eq!(reconcile_absent(&dismissed, SourceOutcome::Read).miss_count, 0);
        // Only an explicit restore brings it back.
        let restored = apply_action(&dismissed, TaskAction::Restore, AFTER).unwrap();
        assert_eq!(restored.state, TaskState::Active);
    }

    // -----------------------------------------------------------------------
    // Pinning
    // -----------------------------------------------------------------------

    #[test]
    fn a_pinned_task_stays_visible_when_it_goes_stale() {
        assert!(!TaskState::Stale.visible(false));
        assert!(TaskState::Stale.visible(true), "a stale pin is exactly when a pin was meant");
        assert!(TaskState::Active.visible(false));
        for hidden in [TaskState::Snoozed, TaskState::Done, TaskState::Dismissed] {
            assert!(!hidden.visible(true), "{hidden:?} is not shown even pinned");
        }
    }

    #[test]
    fn pinning_is_orthogonal_to_every_state() {
        for state in [TaskState::Active, TaskState::Snoozed, TaskState::Done, TaskState::Dismissed, TaskState::Stale] {
            let pinned = apply_action(&task(state), TaskAction::Pin, AFTER).unwrap();
            assert!(pinned.pinned);
            assert_eq!(pinned.state, state, "pinning never changes what a task is");
            let unpinned = apply_action(&pinned, TaskAction::Unpin, AFTER).unwrap();
            assert!(!unpinned.pinned);
            assert_eq!(unpinned.state, state);
        }
    }

    #[test]
    fn a_pin_survives_reconciliation() {
        let pinned = TaskSnapshot { pinned: true, ..TaskSnapshot::active() };
        assert!(reconcile_absent(&pinned, SourceOutcome::Read).pinned);
        assert!(reconcile_present(&pinned, Some("digest-1")).pinned);
    }

    // -----------------------------------------------------------------------
    // Legal and illegal transitions
    // -----------------------------------------------------------------------

    #[test]
    fn every_illegal_transition_is_refused() {
        let illegal = [
            (TaskState::Done, TaskAction::Complete),
            (TaskState::Done, TaskAction::Snooze),
            (TaskState::Done, TaskAction::Dismiss),
            (TaskState::Done, TaskAction::Restore),
            (TaskState::Dismissed, TaskAction::Complete),
            (TaskState::Dismissed, TaskAction::Snooze),
            (TaskState::Dismissed, TaskAction::Dismiss),
            (TaskState::Snoozed, TaskAction::Snooze),
            (TaskState::Active, TaskAction::Restore),
            (TaskState::Stale, TaskAction::Restore),
        ];
        for (state, action) in illegal {
            let error = apply_action(&task(state), action, AFTER).unwrap_err();
            assert_eq!(error.from, state.as_str(), "{state:?} + {action:?}");
            assert!(!error.reason().is_empty());
        }
    }

    #[test]
    fn every_legal_transition_is_allowed() {
        let legal = [
            (TaskState::Active, TaskAction::Complete, TaskState::Done),
            (TaskState::Active, TaskAction::Snooze, TaskState::Snoozed),
            (TaskState::Active, TaskAction::Dismiss, TaskState::Dismissed),
            (TaskState::Snoozed, TaskAction::Complete, TaskState::Done),
            (TaskState::Snoozed, TaskAction::Dismiss, TaskState::Dismissed),
            (TaskState::Snoozed, TaskAction::Restore, TaskState::Active),
            (TaskState::Stale, TaskAction::Complete, TaskState::Done),
            (TaskState::Stale, TaskAction::Snooze, TaskState::Snoozed),
            (TaskState::Stale, TaskAction::Dismiss, TaskState::Dismissed),
            (TaskState::Dismissed, TaskAction::Restore, TaskState::Active),
        ];
        for (from, action, expected) in legal {
            let next = apply_action(&task(from), action, AFTER)
                .unwrap_or_else(|error| panic!("{from:?} + {action:?} should be legal: {error:?}"));
            assert_eq!(next.state, expected);
        }
    }

    #[test]
    fn a_resolution_records_when_it_happened_and_a_restore_clears_it() {
        let current = TaskSnapshot {
            evidence_digest: Some("digest-1".into()),
            ..TaskSnapshot::active()
        };
        let done = apply_action(&current, TaskAction::Complete, RESOLVED).unwrap();
        assert_eq!(done.resolved_at.as_deref(), Some(RESOLVED));
        assert_eq!(done.resolved_evidence_digest.as_deref(), Some("digest-1"));
        let dismissed = apply_action(&TaskSnapshot::active(), TaskAction::Dismiss, RESOLVED).unwrap();
        let restored = apply_action(&dismissed, TaskAction::Restore, AFTER).unwrap();
        assert!(restored.resolved_at.is_none(), "a restored task is not resolved");
        assert!(restored.resolved_evidence_digest.is_none());
    }

    // -----------------------------------------------------------------------
    // Storage vocabulary, snoozes, and ephemerality
    // -----------------------------------------------------------------------

    #[test]
    fn an_unknown_stored_state_reads_as_dismissed() {
        // Fail-closed for a board: hiding a row whose meaning is unknown beats showing one
        // a user might act on.
        for state in [TaskState::Active, TaskState::Snoozed, TaskState::Done, TaskState::Dismissed, TaskState::Stale] {
            assert_eq!(TaskState::parse(state.as_str()), state);
        }
        assert_eq!(TaskState::parse("some_future_state"), TaskState::Dismissed);
        assert_eq!(TaskState::parse(""), TaskState::Dismissed);
    }

    #[test]
    fn a_snooze_expires_only_when_its_deadline_can_be_read_and_has_passed() {
        assert!(snooze_expired(Some(RESOLVED), AFTER));
        assert!(snooze_expired(Some(RESOLVED), RESOLVED), "the deadline itself has arrived");
        assert!(!snooze_expired(Some(AFTER), RESOLVED));
        // An unreadable or absent deadline leaves the snooze in place: expiring early would
        // surprise the user who set it.
        assert!(!snooze_expired(None, AFTER));
        assert!(!snooze_expired(Some("whenever"), AFTER));
        assert!(!snooze_expired(Some(RESOLVED), "whenever"));
    }
}
