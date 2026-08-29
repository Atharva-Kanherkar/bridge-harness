//! Live check polling for pull requests the GitHub surface has already exposed.

use crate::{events::CoreEvent, github_surface::{CheckStatus, PullRequestCheck, PullRequestSummary}, BridgeCore};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

pub const FOCUSED_CADENCE: Duration = Duration::from_secs(15);
pub const UNFOCUSED_CADENCE: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct GithubPoller {
    watched: Mutex<HashMap<(String, u64), WatchedPullRequest>>,
}

#[derive(Clone)]
struct WatchedPullRequest {
    path: PathBuf,
    checks: Option<Vec<PullRequestCheck>>,
}

impl GithubPoller {
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
                watched.insert((workspace_id.into(), pull_request.number), WatchedPullRequest { path: path.clone(), checks });
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
            entry.checks = Some(checks);
            if complete {
                watched_map.remove(&(workspace_id.clone(), number));
            }
            drop(watched_map);
            if changed {
                core.events.publish(CoreEvent::GithubChecksChanged { workspace_id, number });
            }
        }
    }
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

    fn check(status: CheckStatus) -> PullRequestCheck {
        PullRequestCheck {
            name: "build".into(),
            status,
            conclusion: None,
            log_url: String::new(),
            workflow: "ci".into(),
        }
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
}
