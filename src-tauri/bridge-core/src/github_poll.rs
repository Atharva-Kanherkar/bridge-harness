//! Live check polling for pull requests the GitHub surface has already exposed.

use crate::{events::CoreEvent, github_surface::{CheckStatus, PullRequestCheck, PullRequestSummary}, BridgeCore};
use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

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
        // transition per refetch would go unannounced.
        let mut previous = HashMap::new();
        watched.retain(|(workspace, number), entry| {
            if workspace == workspace_id {
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
                    WatchedPullRequest {
                        path: path.clone(),
                        head_branch: pull_request.head_branch.clone(),
                        title: pull_request.title.clone(),
                        checks,
                    },
                );
            }
        }
    }

    fn poll_once(&self, core: &BridgeCore) {
        let pending: Vec<_> = self.watched.lock().unwrap().iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        for ((workspace_id, number), watched) in pending {
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
    fn one_background_refresh_runs_per_workspace() {
        let poller = GithubPoller::default();
        assert!(poller.begin_refresh("ws"));
        assert!(!poller.begin_refresh("ws"), "duplicate refresh is suppressed");
        assert!(poller.begin_refresh("other"), "workspaces refresh independently");
        poller.finish_refresh("ws");
        assert!(poller.begin_refresh("ws"), "completion releases the refresh slot");
    }
}
