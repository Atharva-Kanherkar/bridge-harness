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
        watched.retain(|(workspace, _), _| workspace != workspace_id);
        for pull_request in pull_requests {
            if pull_request.checks.queued > 0 || pull_request.checks.in_progress > 0 {
                watched.insert((workspace_id.into(), pull_request.number), WatchedPullRequest { path: path.clone(), checks: None });
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
            let changed = entry.checks.as_ref().is_some_and(|previous| previous != &checks);
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
}
