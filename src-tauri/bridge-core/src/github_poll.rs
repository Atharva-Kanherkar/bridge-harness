//! Live check polling for pull requests the GitHub surface has already exposed.

use crate::{events::CoreEvent, github_surface::{CheckStatus, PullRequestCheck, PullRequestState, PullRequestSummary}, BridgeCore};
use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::{Arc, Mutex}, time::{Duration, Instant}};

pub const FOCUSED_CADENCE: Duration = Duration::from_secs(15);
pub const UNFOCUSED_CADENCE: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct GithubPoller {
    watched: Mutex<HashMap<(String, u64), WatchedPullRequest>>,
    refreshing: Mutex<HashSet<String>>,
    /// The last terminal check set announced per PR. The list refetch that
    /// follows every checks-changed event re-`watch()`es from a rollup that can
    /// still read "in progress" after the checks completed; without this
    /// baseline that stale re-add would re-observe the same completion and
    /// announce it twice. Keyed forever (bounded by PRs seen): a PR only
    /// re-announces when a *different* terminal check set shows up — a new run.
    announced: Mutex<HashMap<(String, u64), Vec<PullRequestCheck>>>,
}

#[derive(Clone)]
struct WatchedPullRequest {
    path: PathBuf,
    head_branch: String,
    title: String,
    checks: Option<Vec<PullRequestCheck>>,
    /// A chat-attached PR: watched on the card's behalf rather than the
    /// pane's list, kept across list re-fetches, and followed into merged or
    /// closed so the card flips state without waiting for a refocus.
    attached: bool,
    state: Option<PullRequestState>,
    head_sha: Option<String>,
    /// Attached entries poll at the focused cadence while checks run and the
    /// unfocused one once settled; list-watched entries (`None`) poll every
    /// cycle like they always have.
    next_poll: Option<Instant>,
}

impl WatchedPullRequest {
    fn listed(path: PathBuf, pull_request: &PullRequestSummary, checks: Option<Vec<PullRequestCheck>>) -> Self {
        Self {
            path,
            head_branch: pull_request.head_branch.clone(),
            title: pull_request.title.clone(),
            checks,
            attached: false,
            state: None,
            head_sha: None,
            next_poll: None,
        }
    }
}

impl GithubPoller {
    pub(crate) fn begin_refresh(&self, workspace_id: &str) -> bool {
        self.refreshing.lock().unwrap().insert(workspace_id.into())
    }

    pub(crate) fn finish_refresh(&self, workspace_id: &str) {
        self.refreshing.lock().unwrap().remove(workspace_id);
    }

    pub fn watch(&self, workspace_id: &str, path: PathBuf, pull_requests: &[PullRequestSummary]) {
        let mut watched = self.watched.lock().unwrap();
        // Carry each surviving PR's last-seen checks across the re-list. A list
        // refetch happens on every checks-changed event, so resetting baselines
        // here would blind the very next poll's change detection — one real
        // transition per refetch would go unannounced. Chat-attached entries
        // are not the list's business: a merged PR leaves the open list while
        // its card still wants the state flip, so they survive untouched.
        let mut previous = HashMap::new();
        watched.retain(|(workspace, number), entry| {
            if workspace == workspace_id && !entry.attached {
                previous.insert(*number, entry.checks.take());
                false
            } else {
                true
            }
        });
        for pull_request in pull_requests {
            if pull_request.checks.queued > 0 || pull_request.checks.in_progress > 0 {
                let checks = previous.remove(&pull_request.number).flatten();
                watched.insert(
                    (workspace_id.into(), pull_request.number),
                    WatchedPullRequest::listed(path.clone(), pull_request, checks),
                );
            }
        }
    }

    /// Follow one chat-attached PR. Idempotent: a re-arm keeps the baselines
    /// the change detection diffs against.
    pub fn watch_attached(
        &self,
        workspace_id: &str,
        path: PathBuf,
        number: u64,
        head_branch: &str,
        title: &str,
    ) {
        let mut watched = self.watched.lock().unwrap();
        watched
            .entry((workspace_id.into(), number))
            .and_modify(|entry| {
                entry.attached = true;
                entry.path = path.clone();
                entry.head_branch = head_branch.to_owned();
                entry.title = title.to_owned();
            })
            .or_insert_with(|| WatchedPullRequest {
                path,
                head_branch: head_branch.to_owned(),
                title: title.to_owned(),
                checks: None,
                attached: true,
                state: None,
                head_sha: None,
                next_poll: None,
            });
    }

    fn poll_once(&self, core: &BridgeCore) {
        let pending: Vec<_> = self.watched.lock().unwrap().iter()
            .filter(|(_, value)| value.next_poll.is_none_or(|due| due <= Instant::now()))
            .map(|(key, value)| (key.clone(), value.clone())).collect();
        for ((workspace_id, number), watched) in pending {
            if watched.attached {
                self.poll_attached(core, &workspace_id, number, &watched);
                continue;
            }
            core.github_surface.invalidate_checks(&watched.path, number);
            let Ok(checks) = core.github_surface.pr_checks(&watched.path, number) else { continue };
            let complete = all_checks_complete(&checks);
            let mut watched_map = self.watched.lock().unwrap();
            // The entry may have been dropped by a concurrent `watch()` call
            // (e.g. the workspace's PR list was refetched); nothing to update.
            let Some(entry) = watched_map.get_mut(&(workspace_id.clone(), number)) else { continue };
            let changed = rollup_changed(entry.checks.as_deref(), &checks, complete);
            let terminal = complete.then(|| (entry.head_branch.clone(), entry.title.clone()));
            entry.checks = Some(checks.clone());
            if complete {
                watched_map.remove(&(workspace_id.clone(), number));
            }
            drop(watched_map);
            if changed {
                core.events.publish(CoreEvent::GithubChecksChanged {
                    workspace_id: workspace_id.clone(),
                    number,
                });
            }
            if let Some((head_branch, title)) = terminal {
                self.announce_terminal(core, workspace_id, number, head_branch, title, checks);
            }
        }
    }

    /// One poll of a chat-attached PR. On top of the check rollup this tracks
    /// PR state and head SHA, because a card must flip to merged or closed and
    /// must notice a fresh push — both invisible to `pr checks`. Terminal plus
    /// settled (state merged/closed with nothing running) unwatches the PR, so
    /// background polling stays bounded; a slow cadence applies before that so
    /// a later push still surfaces.
    fn poll_attached(&self, core: &BridgeCore, workspace_id: &str, number: u64, watched: &WatchedPullRequest) {
        core.github_surface.invalidate_brief(&watched.path, number);
        core.github_surface.invalidate_checks(&watched.path, number);
        // A transient `gh` failure keeps every last-known value; the card
        // shows its stored snapshot labelled stale and the next cycle retries.
        let Ok(brief) = core.github_surface.pr_brief(&watched.path, number) else { return };
        let checks = core.github_surface.pr_checks(&watched.path, number).unwrap_or_default();
        let checks_active = checks.iter().any(|check| check.status != CheckStatus::Completed);
        let state_terminal = matches!(brief.state, PullRequestState::Closed | PullRequestState::Merged);
        let complete = all_checks_complete(&checks);
        let mut watched_map = self.watched.lock().unwrap();
        let Some(entry) = watched_map.get_mut(&(workspace_id.to_owned(), number)) else { return };
        // Baselines start empty, so the first observation reports only a
        // genuinely completed check set — same rule as the list-watched path.
        let changed = rollup_changed(entry.checks.as_deref(), &checks, complete)
            || entry.state.is_some_and(|state| state != brief.state)
            || entry.head_sha.as_deref().is_some_and(|sha| sha != brief.head_sha);
        let terminal = complete.then(|| (entry.head_branch.clone(), entry.title.clone()));
        entry.checks = Some(checks.clone());
        entry.state = Some(brief.state);
        entry.head_sha = Some(brief.head_sha.clone());
        entry.head_branch = brief.head_branch.clone();
        entry.title = brief.title.clone();
        entry.next_poll = Some(Instant::now() + if checks_active { FOCUSED_CADENCE } else { UNFOCUSED_CADENCE });
        if state_terminal && !checks_active {
            watched_map.remove(&(workspace_id.to_owned(), number));
        }
        drop(watched_map);
        if changed {
            core.events.publish(CoreEvent::GithubChecksChanged {
                workspace_id: workspace_id.to_owned(),
                number,
            });
        }
        if let Some((head_branch, title)) = terminal {
            self.announce_terminal(core, workspace_id.to_owned(), number, head_branch, title, checks);
        }
    }

    /// Publish `GithubCiFinished` for a terminal check set, at most once per
    /// set. See `announced` for why the re-check is needed at all.
    fn announce_terminal(
        &self,
        core: &BridgeCore,
        workspace_id: String,
        number: u64,
        head_branch: String,
        title: String,
        checks: Vec<PullRequestCheck>,
    ) {
        let key = (workspace_id.clone(), number);
        let mut announced = self.announced.lock().unwrap();
        if announced.get(&key) == Some(&checks) {
            return;
        }
        let (failed, total) = summarize(&checks);
        announced.insert(key, checks);
        drop(announced);
        core.events.publish(CoreEvent::GithubCiFinished {
            workspace_id,
            number,
            head_branch,
            title,
            failed,
            total,
        });
    }
}

/// The notification's headline numbers: how many checks ended badly, out of
/// how many. "Badly" mirrors the rerun affordance — failure, timeout, or a
/// startup failure; cancelled and skipped runs are not news.
fn summarize(checks: &[PullRequestCheck]) -> (u32, u32) {
    use crate::github_surface::CheckConclusion;
    let failed = checks
        .iter()
        .filter(|check| {
            matches!(
                check.conclusion,
                Some(CheckConclusion::Failure)
                    | Some(CheckConclusion::TimedOut)
                    | Some(CheckConclusion::StartupFailure)
            )
        })
        .count() as u32;
    (failed, checks.len() as u32)
}

/// An empty result can mean the checks API has not yet caught up with the
/// rollup that put this PR on the watch list (the two are backed by separate
/// `gh` calls), so it must not be read as vacuously "all complete".
fn all_checks_complete(checks: &[PullRequestCheck]) -> bool {
    !checks.is_empty() && checks.iter().all(|check| check.status == CheckStatus::Completed)
}

/// Whether this observation is news worth publishing. With a baseline, any
/// difference is. Without one — the first poll after `watch()` — the PR was
/// pending when the rollup put it on the list, so *completion* is a real
/// transition even though there is nothing to diff against; a PR whose checks
/// finish between the list fetch and the first poll must not be silently
/// dropped from the watch list with its badge still reading "running".
fn rollup_changed(previous: Option<&[PullRequestCheck]>, next: &[PullRequestCheck], complete: bool) -> bool {
    previous.map_or(complete, |previous| previous != next)
}

pub fn start_github_poll_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        core.github_poller.poll_once(&core);
        std::thread::sleep(FOCUSED_CADENCE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_surface::CheckConclusion;

    fn check(status: CheckStatus) -> PullRequestCheck {
        PullRequestCheck {
            name: "build".into(),
            status,
            conclusion: None,
            log_url: String::new(),
            workflow: "ci".into(),
        }
    }

    fn finished(name: &str, conclusion: CheckConclusion) -> PullRequestCheck {
        PullRequestCheck {
            name: name.into(),
            status: CheckStatus::Completed,
            conclusion: Some(conclusion),
            log_url: String::new(),
            workflow: "ci".into(),
        }
    }

    #[test]
    fn a_terminal_check_set_announces_exactly_once_across_reobservation() {
        let scratch = tempfile::tempdir().unwrap();
        let core = crate::runtime::BridgeCore::for_tests(scratch.path());
        let mut events = core.events.subscribe();
        let poller = GithubPoller::default();
        let done = vec![
            finished("build", CheckConclusion::Failure),
            finished("test", CheckConclusion::Success),
        ];
        poller.announce_terminal(&core, "ws".into(), 7, "feat/x".into(), "Title".into(), done.clone());
        // A reconnect refetches the PR list from a rollup that can still read
        // "in progress"; the stale re-watch then re-observes the same terminal
        // set. That path must be silent.
        poller.announce_terminal(&core, "ws".into(), 7, "feat/x".into(), "Title".into(), done);
        match events.try_recv().expect("first terminal set announces") {
            CoreEvent::GithubCiFinished { workspace_id, number, head_branch, failed, total, .. } => {
                assert_eq!(workspace_id, "ws");
                assert_eq!(number, 7);
                assert_eq!(head_branch, "feat/x");
                assert_eq!(failed, 1);
                assert_eq!(total, 2);
            }
            other => panic!("expected GithubCiFinished, got {other:?}"),
        }
        assert!(events.try_recv().is_err(), "the re-observation is deduplicated");
    }

    #[test]
    fn a_different_terminal_set_is_a_new_run_and_announces_again() {
        let scratch = tempfile::tempdir().unwrap();
        let core = crate::runtime::BridgeCore::for_tests(scratch.path());
        let mut events = core.events.subscribe();
        let poller = GithubPoller::default();
        poller.announce_terminal(
            &core, "ws".into(), 7, "feat/x".into(), "Title".into(),
            vec![finished("build", CheckConclusion::Failure)],
        );
        poller.announce_terminal(
            &core, "ws".into(), 7, "feat/x".into(), "Title".into(),
            vec![finished("build", CheckConclusion::Success)],
        );
        assert!(matches!(events.try_recv(), Ok(CoreEvent::GithubCiFinished { failed: 1, .. })));
        assert!(
            matches!(events.try_recv(), Ok(CoreEvent::GithubCiFinished { failed: 0, .. })),
            "a rerun that flips the outcome is news again",
        );
    }

    #[test]
    fn summarize_counts_rerunnable_conclusions_only() {
        let checks = [
            finished("a", CheckConclusion::Failure),
            finished("b", CheckConclusion::TimedOut),
            finished("c", CheckConclusion::StartupFailure),
            finished("d", CheckConclusion::Cancelled),
            finished("e", CheckConclusion::Skipped),
            finished("f", CheckConclusion::Success),
        ];
        assert_eq!(summarize(&checks), (3, 6));
    }

    #[test]
    fn an_empty_result_is_not_treated_as_complete() {
        // A watched PR only ever has an empty `pr_checks` result because of an
        // eventual-consistency gap right after it was added to the watch list
        // (the rollup that triggered watching already saw pending checks), not
        // because it genuinely has zero checks.
        assert!(!all_checks_complete(&[]));
    }

    #[test]
    fn all_completed_checks_are_complete() {
        assert!(all_checks_complete(&[check(CheckStatus::Completed), check(CheckStatus::Completed)]));
    }

    #[test]
    fn a_pending_check_is_not_complete() {
        assert!(!all_checks_complete(&[check(CheckStatus::Completed), check(CheckStatus::InProgress)]));
    }

    #[test]
    fn completion_on_the_first_observation_is_published() {
        // The watch-time rollup said pending; all-complete now is a real
        // transition even with no stored baseline to diff against.
        let done = [check(CheckStatus::Completed)];
        assert!(rollup_changed(None, &done, true));
    }

    #[test]
    fn a_pending_first_observation_is_not_news() {
        let running = [check(CheckStatus::InProgress)];
        assert!(!rollup_changed(None, &running, false));
    }

    #[test]
    fn a_baseline_difference_is_published_and_equality_is_not() {
        let running = vec![check(CheckStatus::InProgress)];
        let done = [check(CheckStatus::Completed)];
        assert!(rollup_changed(Some(&running), &done, true));
        assert!(!rollup_changed(Some(&running), &running.clone(), false));
    }

    #[test]
    fn a_relist_keeps_the_baseline_for_surviving_pull_requests() {
        use crate::github_surface::{CheckRollup, Mergeability, PullRequestState, PullRequestSummary, ReviewDecision};
        fn pending_summary(number: u64) -> PullRequestSummary {
            PullRequestSummary {
                number,
                title: "t".into(),
                state: PullRequestState::Open,
                is_draft: false,
                author: None,
                head_branch: "b".into(),
                review_decision: ReviewDecision::None,
                mergeability: Mergeability::Mergeable,
                merge_state_status: String::new(),
                checks: CheckRollup { in_progress: 1, total: 1, ..Default::default() },
                url: String::new(),
            }
        }
        let poller = GithubPoller::default();
        let path = PathBuf::from("/tmp/repo");
        poller.watch("ws", path.clone(), &[pending_summary(7)]);
        poller
            .watched
            .lock()
            .unwrap()
            .get_mut(&("ws".into(), 7))
            .unwrap()
            .checks = Some(vec![check(CheckStatus::InProgress)]);
        // The panel refetches the list on every checks-changed event; that
        // refetch must not blind the next poll's change detection.
        poller.watch("ws", path, &[pending_summary(7)]);
        let watched = poller.watched.lock().unwrap();
        assert_eq!(
            watched.get(&("ws".into(), 7)).unwrap().checks,
            Some(vec![check(CheckStatus::InProgress)]),
        );
    }

    #[test]
    fn attached_entries_survive_a_list_rewatch_with_their_baselines() {
        let poller = GithubPoller::default();
        let path = PathBuf::from("/tmp/repo");
        poller.watch_attached("ws", path.clone(), 9, "feat/card", "Card PR");
        poller
            .watched
            .lock()
            .unwrap()
            .get_mut(&("ws".into(), 9))
            .unwrap()
            .checks = Some(vec![check(CheckStatus::InProgress)]);
        // The pane refetches its list; the attached PR (merged, say, and so
        // absent from the open list) must keep being followed for its card.
        poller.watch("ws", path, &[]);
        let watched = poller.watched.lock().unwrap();
        let entry = watched.get(&("ws".into(), 9)).expect("attached entry survives");
        assert!(entry.attached);
        assert_eq!(entry.checks, Some(vec![check(CheckStatus::InProgress)]));
    }

    #[test]
    fn watch_attached_is_idempotent_and_keeps_baselines() {
        let poller = GithubPoller::default();
        poller.watch_attached("ws", PathBuf::from("/tmp/repo"), 9, "feat/card", "Card PR");
        poller
            .watched
            .lock()
            .unwrap()
            .get_mut(&("ws".into(), 9))
            .unwrap()
            .checks = Some(vec![check(CheckStatus::Completed)]);
        poller.watch_attached("ws", PathBuf::from("/tmp/repo"), 9, "feat/card", "Card PR");
        let watched = poller.watched.lock().unwrap();
        assert_eq!(watched.len(), 1, "re-arming never duplicates the watch");
        assert_eq!(
            watched.get(&("ws".into(), 9)).unwrap().checks,
            Some(vec![check(CheckStatus::Completed)]),
            "the change-detection baseline survives a re-arm",
        );
    }

    #[test]
    fn an_attached_pr_is_retried_after_a_transient_gh_failure() {
        let scratch = tempfile::tempdir().unwrap();
        // `for_tests` has no `gh`: every read fails, which is exactly the
        // transient-outage shape the card must ride through.
        let core = crate::runtime::BridgeCore::for_tests(scratch.path());
        let poller = GithubPoller::default();
        poller.watch_attached("ws", scratch.path().to_path_buf(), 9, "feat/card", "Card PR");
        poller.poll_once(&core);
        let watched = poller.watched.lock().unwrap();
        let entry = watched.get(&("ws".into(), 9)).expect("a failed poll keeps the watch");
        assert!(
            entry.next_poll.is_none(),
            "a failed cycle leaves the entry due immediately so the next cycle retries",
        );
    }

    #[test]
    fn one_background_refresh_runs_per_workspace() {
        let poller = GithubPoller::default();
        assert!(poller.begin_refresh("ws"));
        assert!(!poller.begin_refresh("ws"), "duplicate refresh is suppressed");
        assert!(poller.begin_refresh("other"), "workspaces refresh independently");
        poller.finish_refresh("ws");
        assert!(poller.begin_refresh("ws"), "completion releases the refresh slot");
    }
}
