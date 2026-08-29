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
            let complete = checks.iter().all(|check| check.status == CheckStatus::Completed);
            let changed = self.watched.lock().unwrap().get(&(workspace_id.clone(), number)).and_then(|value| value.checks.as_ref()).is_some_and(|previous| previous != &checks);
            if let Some(entry) = self.watched.lock().unwrap().get_mut(&(workspace_id.clone(), number)) { entry.checks = Some(checks); }
            if changed { core.events.publish(CoreEvent::GithubChecksChanged { workspace_id: workspace_id.clone(), number }); }
            if complete { self.watched.lock().unwrap().remove(&(workspace_id, number)); }
        }
    }
}

pub fn start_github_poll_maintenance(core: Arc<BridgeCore>) {
    std::thread::spawn(move || loop {
        core.github_poller.poll_once(&core);
        std::thread::sleep(FOCUSED_CADENCE);
    });
}
