//! Auto-review on push (#1121): with `autoReview` on, a pull request that
//! opens or gets new commits is reviewed on its own.
//!
//! Only repositories the person already looks at are read: those whose GitHub
//! pane listed pull requests since Bridge started, and those that already
//! have review jobs. Each repository is listed once a minute (number, head,
//! draft, author), and only while auto-review is on. Two workspaces of one
//! repository share one watch: a PR head is reviewed once, whichever
//! workspace saw it.
//!
//! The first listing is a baseline, so turning auto-review on (or starting
//! Bridge) never reviews every open PR at once. After it, a new head or a PR
//! that appears waits [`DEBOUNCE`] since the last change seen, then one job
//! starts at the newest head. One exception on the baseline: a PR whose latest
//! review was started by a push and covered an older head was pushed while
//! Bridge was closed, so it is reviewed again.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bridge_protocol::messages::{self as wire, ReviewAutoMode, ReviewJobSettings};

use crate::github_surface::{GithubRepository, OpenPullRequestHead, OPEN_HEADS_LIMIT};
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
    /// Bumped by every save that changes auto-review's settings, under the
    /// store lock. A round from an older generation drops what it saw and
    /// starts nothing, so Off then On between two polls is a fresh start.
    generation: AtomicU64,
}

#[derive(Debug, Default)]
struct State {
    /// The settings generation the watches were built under.
    generation: u64,
    /// The filters the watches were built under. A change starts a new
    /// baseline, so switching "My PRs" to "All PRs" reviews nothing at once.
    filters: Option<(ReviewAutoMode, bool)>,
    /// One per repository (`host/owner/name`, lowercase).
    watches: HashMap<String, (Target, Watch)>,
    /// The signed-in login per GitHub host, and when it was read.
    viewers: HashMap<String, (Option<String>, Instant)>,
}

/// Where a repository's watch lists from and starts reviews in: the first
/// of its workspaces, by id.
#[derive(Debug, Clone, PartialEq)]
struct Target {
    workspace_id: String,
    path: PathBuf,
    repository: GithubRepository,
}

fn repository_key(repository: &GithubRepository) -> String {
    format!("{}/{}/{}", repository.host, repository.owner, repository.name).to_lowercase()
}

/// One workspace's open pull requests as last listed.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Watch {
    pub next_list: Option<Instant>,
    /// Whether the first listing (the baseline) has been seen.
    pub baselined: bool,
    /// The highest PR number any listing has shown. GitHub numbers rise, so
    /// only a PR above it was opened since; any other unseen one was out of
    /// view (past a partial listing, or closed and reopened).
    pub highest_seen: Option<u64>,
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

/// Fold one listing into a repository's watch. A new head, a PR that
/// appears after the baseline, or one that becomes reviewable (a draft
/// marked ready) waits from now. A PR no longer listed (closed or merged) is
/// forgotten. `catch_up` decides, on the baseline only, whether a PR was
/// pushed while Bridge was closed.
///
/// A PR this watch has not seen counts as newly opened only when its number
/// is above every PR any listing has shown ([`Watch::highest_seen`]). A
/// listing that is not `complete` (it hit [`OPEN_HEADS_LIMIT`]) also keeps
/// the PRs it left out.
pub fn observe(
    watch: &mut Watch,
    listing: &[OpenPullRequestHead],
    complete: bool,
    filters: &Filters,
    now: Instant,
    catch_up: impl Fn(u64, &str) -> bool,
) {
    let baseline = !watch.baselined;
    let highest_seen = watch.highest_seen;
    let mut next = HashMap::new();
    for pull_request in listing {
        let eligible = filters.takes(pull_request);
        let pending_since = match watch.prs.get(&pull_request.number) {
            None if baseline => (eligible && catch_up(pull_request.number, &pull_request.head_sha)).then_some(now),
            None if highest_seen.is_some_and(|highest| pull_request.number <= highest) => None,
            None => eligible.then_some(now),
            Some(seen) if seen.head_sha != pull_request.head_sha => eligible.then_some(now),
            Some(seen) if eligible && !seen.eligible => Some(now),
            Some(seen) => seen.pending_since.filter(|_| eligible),
        };
        next.insert(pull_request.number, Seen { head_sha: pull_request.head_sha.clone(), eligible, pending_since });
    }
    if !complete {
        for (number, seen) in watch.prs.drain() {
            next.entry(number).or_insert(seen);
        }
    }
    watch.prs = next;
    watch.baselined = true;
    watch.highest_seen = listing.iter().map(|pull_request| pull_request.number).chain(highest_seen).max();
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

    /// Auto-review's settings changed: the next round starts afresh. Called
    /// with the store lock held, right after the save.
    pub fn settings_changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The live workspaces watched, with their paths as the store has them.
    /// One archived (its row is gone) leaves the watch at once, so another
    /// workspace of its repository takes over.
    fn watched(&self, core: &BridgeCore) -> Vec<(String, PathBuf)> {
        let (live, reviewed): (HashMap<String, String>, Vec<String>) = {
            let db = super::lock_db(core);
            let live = db
                .prepare("SELECT id, path FROM workspaces WHERE path IS NOT NULL")
                .and_then(|mut statement| {
                    statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<HashMap<_, _>, _>>()
                })
                .unwrap_or_default();
            (live, super::store::reviewed_workspaces(&db).unwrap_or_default())
        };
        let mut exposed = self.exposed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        exposed.retain(|workspace_id, _| live.contains_key(workspace_id));
        let mut watched: Vec<(String, PathBuf)> = exposed
            .keys()
            .chain(reviewed.iter())
            .filter_map(|workspace_id| live.get(workspace_id).map(|path| (workspace_id.clone(), PathBuf::from(path))))
            .collect();
        watched.sort();
        watched.dedup();
        watched
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
    let generation = auto.generation.load(Ordering::SeqCst);
    if state.generation != generation {
        // The settings changed since the last round, perhaps more than once
        // (Off, then On again): nothing seen under the old ones counts.
        *state = State { generation, ..State::default() };
    }
    if current.auto_review == ReviewAutoMode::Off {
        // Off forgets everything: turning it on again starts a new baseline.
        *state = State { generation, ..State::default() };
        return;
    }
    let filters_key = (current.auto_review, current.skip_draft_prs);
    if state.filters != Some(filters_key) {
        state.watches.clear();
        state.filters = Some(filters_key);
    }
    // One watch per repository, owned by the first of its workspaces.
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for (workspace_id, path) in auto.watched(core) {
        let Ok(repository) = core.github_surface.resolve_repository(&path) else { continue };
        targets.entry(repository_key(&repository)).or_insert(Target { workspace_id, path, repository });
    }
    state.watches.retain(|key, _| targets.contains_key(key));

    for (key, target) in &targets {
        let (owner, watch) = state.watches.entry(key.clone()).or_insert_with(|| (target.clone(), Watch::default()));
        *owner = target.clone();
        if watch.next_list.is_some_and(|next| next > now) {
            continue;
        }
        watch.next_list = Some(now + LIST_EVERY);
        let listing = match core.github_surface.list_open_heads(&target.path) {
            Ok(listing) => listing,
            Err(error) => {
                eprintln!("[reviews] auto-review could not list the pull requests of {key}: {error}");
                continue;
            }
        };
        let complete = listing.len() < OPEN_HEADS_LIMIT;
        if !complete {
            eprintln!("[reviews] auto-review read the first {OPEN_HEADS_LIMIT} open pull requests of {key}; older ones keep what was seen");
        }
        let viewer = match current.auto_review {
            ReviewAutoMode::Mine => match viewer(core, &mut state.viewers, target, now) {
                Some(login) => Some(login),
                // Not knowing who "me" is, no baseline: otherwise the first
                // read that works would make every PR of theirs look new.
                None => continue,
            },
            _ => None,
        };
        let filters = Filters { mode: current.auto_review, skip_drafts: current.skip_draft_prs, viewer };
        let latest: HashMap<u64, (wire::ReviewTrigger, String)> = {
            let db = super::lock_db(core);
            super::store::latest_jobs(&db, &target.repository.host, &target.repository.owner, &target.repository.name)
                .unwrap_or_default()
        };
        let (_, watch) = state.watches.entry(key.clone()).or_insert_with(|| (target.clone(), Watch::default()));
        observe(watch, &listing, complete, &filters, now, |number, head| {
            latest.get(&number).is_some_and(|(trigger, reviewed)| *trigger == wire::ReviewTrigger::Push && reviewed != head)
        });
    }

    let mut starts = Vec::new();
    for (key, (target, watch)) in &mut state.watches {
        for (number, _) in due(watch, now) {
            starts.push((key.clone(), target.clone(), number));
        }
    }
    for (key, target, number) in starts {
        let viewer = state.viewers.get(&target.repository.host.to_lowercase()).and_then(|(login, _)| login.clone());
        let filters = Filters { mode: current.auto_review, skip_drafts: current.skip_draft_prs, viewer };
        let started = start_one(core, &target, number, &filters, generation);
        let Some((_, watch)) = state.watches.get_mut(&key) else { continue };
        let Some(seen) = watch.prs.get_mut(&number) else { continue };
        match started {
            // The job reviews the head GitHub has now, which may be newer
            // than the listing's: that one is not a change to review again.
            Ok(Started::Reviewing(head)) => seen.head_sha = head,
            // A draft now, or someone else's: when a listing shows it
            // reviewable again, that is a change.
            Ok(Started::NotNow) => seen.eligible = false,
            Ok(Started::Refused) => {}
            Err(error) => {
                eprintln!("[reviews] auto-review could not start a review of {key}#{number}: {error}");
                // Try again after another quiet period.
                seen.pending_since = Some(now);
            }
        }
    }
}

/// The signed-in login on the target's host. A read is trusted for
/// [`VIEWER_TTL`] from when it was made; a failed read is tried again next
/// time.
fn viewer(core: &BridgeCore, viewers: &mut HashMap<String, (Option<String>, Instant)>, target: &Target, now: Instant) -> Option<String> {
    let host = target.repository.host.to_lowercase();
    if let Some((Some(login), read)) = viewers.get(&host) {
        if now.duration_since(*read) < VIEWER_TTL {
            return Some(login.clone());
        }
    }
    let login = core.github_surface.viewer_login(&target.path).ok();
    viewers.insert(host, (login.clone(), now));
    login
}

/// What a start came to.
enum Started {
    /// A job reviews this head.
    Reviewing(String),
    /// GitHub says the PR is not one to review now (a draft, or not theirs).
    NotNow,
    /// The store said no: auto-review was turned off or changed, or that
    /// head is already reviewed.
    Refused,
}

/// Start the push review of one PR, judged on what GitHub says now: a PR that
/// became a draft, or is someone else's, is not started. The last word is the
/// store's: under its lock, with the settings as they are then, auto-review
/// must still be on with the same filters and unchanged since this round
/// began, and no workspace may have reviewed that head already. A closed or
/// merged PR is never started.
fn start_one(core: &Arc<BridgeCore>, target: &Target, number: u64, filters: &Filters, generation: u64) -> Result<Started, crate::BridgeError> {
    let fresh = core
        .github_surface
        .pr_head(&target.path, number)
        .map_err(|error| crate::BridgeError::Invalid(error.to_string()))?;
    let now_open = OpenPullRequestHead {
        number,
        head_sha: fresh.head_sha.clone(),
        is_draft: fresh.is_draft,
        author: fresh.author.clone(),
    };
    if !fresh.is_open || !filters.takes(&now_open) {
        return Ok(Started::NotNow);
    }
    let params = wire::StartReviewParams {
        workspace_id: target.workspace_id.clone(),
        number,
        harnesses: None,
        trigger: Some(wire::ReviewTrigger::Push),
    };
    let head_sha = fresh.head_sha.clone();
    let gate = |db: &rusqlite::Connection, settings: &ReviewJobSettings, pr: super::store::PrKey<'_>| {
        core.reviews.auto.generation.load(Ordering::SeqCst) == generation
            && settings.auto_review == filters.mode
            && settings.skip_draft_prs == filters.skip_drafts
            && !super::store::has_review_of_head(db, pr, &head_sha).unwrap_or(true)
    };
    let head = super::PrHead {
        repo_host: fresh.repository.host,
        repo_owner: fresh.repository.owner,
        repo_name: fresh.repository.name,
        head_sha: fresh.head_sha,
        base_sha: fresh.base_sha,
    };
    Ok(match super::start_gated(core, &params, head, &gate)? {
        Some(job) => Started::Reviewing(job.head_sha),
        None => Started::Refused,
    })
}

#[cfg(test)]
#[path = "auto_tests.rs"]
mod tests;
