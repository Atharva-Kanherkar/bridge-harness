//! Auto-review on push (#1121): with `autoReview` on, a pull request that
//! opens or gets new commits is reviewed on its own.
//!
//! Only repositories the person already looks at are read: those whose GitHub
//! pane listed pull requests since Bridge started, and those that already
//! have review jobs. Each is listed once a minute (number, head, draft,
//! author), and only while auto-review is on.
//!
//! The first listing is a baseline, so turning auto-review on (or starting
//! Bridge) never reviews every open PR at once. After it, a new head or a PR
//! that appears waits [`DEBOUNCE`] since the last change seen, then one job
//! starts at the newest head. One exception on the baseline: a PR whose latest
//! review was started by a push and covered an older head was pushed while
//! Bridge was closed, so it is reviewed again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bridge_protocol::messages::{self as wire, ReviewAutoMode, ReviewJobSettings};

use crate::github_surface::OpenPullRequestHead;
use crate::runtime::BridgeCore;

/// How often a watched repository's open pull requests are read.
pub const LIST_EVERY: Duration = Duration::from_secs(60);
/// How long a PR must stay unchanged before its review starts, so a burst of
/// pushes becomes one review of the last one.
pub const DEBOUNCE: Duration = Duration::from_secs(120);
/// How long the signed-in login is trusted before it is read again.
const VIEWER_TTL: Duration = Duration::from_secs(3600);

/// Auto-review's state, kept in memory: nothing here needs to survive a
/// restart (see the baseline rule above).
#[derive(Debug, Default)]
pub struct AutoReview {
    /// Workspaces whose GitHub pane listed pull requests since start. Its own
    /// lock, so the pane never waits on a slow `gh` read of the tick.
    exposed: Mutex<HashMap<String, PathBuf>>,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The filters the watches were built under. A change starts a new
    /// baseline, so switching "My PRs" to "All PRs" reviews nothing at once.
    filters: Option<(ReviewAutoMode, bool)>,
    watches: HashMap<String, Watch>,
    viewer: Option<(Option<String>, Instant)>,
}

/// One workspace's open pull requests as last listed.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Watch {
    pub next_list: Option<Instant>,
    /// Whether the first listing (the baseline) has been seen.
    pub baselined: bool,
    pub prs: HashMap<u64, Seen>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub head_sha: String,
    /// Whether the filters let it be reviewed.
    pub eligible: bool,
    /// When the change waiting to be reviewed was last seen.
    pub pending_since: Option<Instant>,
}

/// Which pull requests auto-review takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filters {
    pub mode: ReviewAutoMode,
    pub skip_drafts: bool,
    /// The signed-in login; unknown means "My PRs" takes none.
    pub viewer: Option<String>,
}

impl Filters {
    pub fn takes(&self, pull_request: &OpenPullRequestHead) -> bool {
        let author = match self.mode {
            ReviewAutoMode::Off => false,
            ReviewAutoMode::All => true,
            ReviewAutoMode::Mine => self
                .viewer
                .as_deref()
                .is_some_and(|viewer| pull_request.author.as_deref().is_some_and(|author| author.eq_ignore_ascii_case(viewer))),
        };
        author && !(self.skip_drafts && pull_request.is_draft)
    }
}

/// Fold one listing into a workspace's watch. A new head, a PR that appears
/// after the baseline, or one that becomes reviewable (a draft marked ready)
/// waits from now. A PR no longer listed (closed or merged) is forgotten.
/// `catch_up` decides, on the baseline only, whether a PR was pushed while
/// Bridge was closed.
pub fn observe(
    watch: &mut Watch,
    listing: &[OpenPullRequestHead],
    filters: &Filters,
    now: Instant,
    catch_up: impl Fn(u64, &str) -> bool,
) {
    let baseline = !watch.baselined;
    let mut next = HashMap::new();
    for pull_request in listing {
        let eligible = filters.takes(pull_request);
        let pending_since = match watch.prs.get(&pull_request.number) {
            None if baseline => (eligible && catch_up(pull_request.number, &pull_request.head_sha)).then_some(now),
            None => eligible.then_some(now),
            Some(seen) if seen.head_sha != pull_request.head_sha => eligible.then_some(now),
            Some(seen) if eligible && !seen.eligible => Some(now),
            Some(seen) => seen.pending_since.filter(|_| eligible),
        };
        next.insert(pull_request.number, Seen { head_sha: pull_request.head_sha.clone(), eligible, pending_since });
    }
    watch.prs = next;
    watch.baselined = true;
}

/// The PRs whose change has been quiet for [`DEBOUNCE`], with the head to
/// review. Each is handed out once: it waits again only on a new change.
pub fn due(watch: &mut Watch, now: Instant) -> Vec<(u64, String)> {
    let mut ready = Vec::new();
    for (number, seen) in &mut watch.prs {
        if seen.pending_since.is_some_and(|since| now.duration_since(since) >= DEBOUNCE) {
            seen.pending_since = None;
            ready.push((*number, seen.head_sha.clone()));
        }
    }
    ready.sort();
    ready
}

impl AutoReview {
    /// The GitHub pane listed this workspace's pull requests: auto-review may
    /// watch it.
    pub fn expose(&self, workspace_id: &str, path: PathBuf) {
        self.exposed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(workspace_id.to_owned(), path);
    }

    /// The workspaces watched, with their paths.
    fn watched(&self, core: &BridgeCore) -> Vec<(String, PathBuf)> {
        let mut workspaces: HashMap<String, PathBuf> =
            self.exposed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let reviewed: Vec<String> = {
            let db = super::lock_db(core);
            super::store::reviewed_workspaces(&db).unwrap_or_default()
        };
        for workspace_id in reviewed {
            if workspaces.contains_key(&workspace_id) {
                continue;
            }
            if let Ok(path) = core.workspace_path(&workspace_id) {
                workspaces.insert(workspace_id, PathBuf::from(path));
            }
        }
        let mut sorted: Vec<_> = workspaces.into_iter().collect();
        sorted.sort();
        sorted
    }
}

fn settings(core: &BridgeCore) -> Option<ReviewJobSettings> {
    super::settings::load(&super::lock_db(core)).ok()
}

/// One round: list the watched repositories that are due, then start the
/// reviews whose debounce has passed. Runs on the GitHub poll thread.
pub fn tick(core: &Arc<BridgeCore>, now: Instant) {
    let auto = &core.reviews.auto;
    let Some(current) = settings(core) else { return };
    let mut state = auto.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if current.auto_review == ReviewAutoMode::Off {
        // Off forgets everything: turning it on again starts a new baseline.
        *state = State::default();
        return;
    }
    let filters_key = (current.auto_review, current.skip_draft_prs);
    if state.filters != Some(filters_key) {
        state.watches.clear();
        state.filters = Some(filters_key);
    }
    let watched = auto.watched(core);
    let keep: HashSet<&String> = watched.iter().map(|(id, _)| id).collect();
    state.watches.retain(|workspace_id, _| keep.contains(workspace_id));

    for (workspace_id, path) in &watched {
        let watch = state.watches.entry(workspace_id.clone()).or_default();
        if watch.next_list.is_some_and(|next| next > now) {
            continue;
        }
        watch.next_list = Some(now + LIST_EVERY);
        let listing = match core.github_surface.list_open_heads(path) {
            Ok(listing) => listing,
            Err(error) => {
                eprintln!("[reviews] auto-review could not list the pull requests of {workspace_id}: {error}");
                continue;
            }
        };
        let viewer = if current.auto_review == ReviewAutoMode::Mine {
            let fresh = state.viewer.as_ref().filter(|(login, read)| login.is_some() && now.duration_since(*read) < VIEWER_TTL);
            let login = match fresh {
                Some((login, _)) => login.clone(),
                None => core.github_surface.viewer_login(path).ok(),
            };
            state.viewer = Some((login.clone(), now));
            if login.is_none() {
                // Not knowing who "me" is, no baseline: otherwise the first
                // read that works would make every PR of theirs look new.
                continue;
            }
            login
        } else {
            None
        };
        let filters = Filters { mode: current.auto_review, skip_drafts: current.skip_draft_prs, viewer };
        let latest: HashMap<u64, (wire::ReviewTrigger, String)> = {
            let db = super::lock_db(core);
            super::store::latest_jobs(&db, workspace_id).unwrap_or_default()
        };
        let watch = state.watches.entry(workspace_id.clone()).or_default();
        observe(watch, &listing, &filters, now, |number, head| {
            latest.get(&number).is_some_and(|(trigger, reviewed)| *trigger == wire::ReviewTrigger::Push && reviewed != head)
        });
    }

    let mut starts = Vec::new();
    for (workspace_id, watch) in &mut state.watches {
        for (number, head) in due(watch, now) {
            starts.push((workspace_id.clone(), number, head));
        }
    }
    for (workspace_id, number, head) in starts {
        // Read again right before starting: a save that turned it off wins.
        if settings(core).is_none_or(|settings| settings.auto_review == ReviewAutoMode::Off) {
            *state = State::default();
            return;
        }
        let reviewed = super::store::has_review_of_head(&super::lock_db(core), &workspace_id, number, &head).unwrap_or(true);
        if reviewed {
            continue;
        }
        let params = wire::StartReviewParams {
            workspace_id: workspace_id.clone(),
            number,
            harnesses: None,
            trigger: Some(wire::ReviewTrigger::Push),
        };
        match super::start(core, &params) {
            Ok(job) => {
                // The job reviews the head GitHub has now, which may be newer
                // than the listing's: that one is not a change to review again.
                if let Some(seen) = state.watches.get_mut(&workspace_id).and_then(|watch| watch.prs.get_mut(&number)) {
                    seen.head_sha = job.head_sha;
                }
            }
            Err(error) => {
                eprintln!("[reviews] auto-review could not start a review of #{number} in {workspace_id}: {error}");
                // Try again after another quiet period.
                if let Some(seen) = state.watches.get_mut(&workspace_id).and_then(|watch| watch.prs.get_mut(&number)) {
                    seen.pending_since = Some(now);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "auto_tests.rs"]
mod tests;
