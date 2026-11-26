//! Repository observations are prepared before a forest write takes the
//! database guard. Streaming writes only read the cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::{store, store::SessionRepositoryLocation, BridgeError};

const DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Slot {
    stamp: Option<(Instant, Value)>,
    invalidated_at: Option<Instant>,
}

#[derive(Default)]
struct Cache {
    worktrees: HashMap<PathBuf, Slot>,
    locations: HashMap<SessionRepositoryLocation, Option<PathBuf>>,
    cold_refreshes: HashMap<SessionRepositoryLocation, Instant>,
    worker_running: bool,
}

static CACHE: std::sync::LazyLock<Mutex<Cache>> =
    std::sync::LazyLock::new(|| Mutex::new(Cache::default()));
static REFRESH: Condvar = Condvar::new();

fn cache() -> MutexGuard<'static, Cache> {
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn unavailable() -> Value {
    json!({"status":"unavailable"})
}

pub(crate) fn exact_kind(kind: &str) -> bool {
    matches!(
        kind,
        "user.message"
            | "assistant.message"
            | "turn.completed"
            | "turn.failed"
            | "checkpoint"
            | "branch.summary"
            | "compaction"
            | crate::agent::NATIVE_COMPACTION_KIND
    )
}

pub(crate) fn changes_folder(kind: &str) -> bool {
    kind.starts_with("file_change.")
        || matches!(
            kind,
            "command.completed" | "tool.completed" | "diff.updated" | "subagent.completed"
        )
}

pub(crate) struct Prepared {
    location: Option<SessionRepositoryLocation>,
    value: Value,
}

impl Prepared {
    pub(crate) fn for_session(
        db: &Mutex<Connection>,
        session_id: &str,
    ) -> Result<Self, BridgeError> {
        let location = store::repository_location_for_session(
            &db.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            session_id,
        )?;
        Ok(Self::observe(location))
    }

    pub(crate) fn observe(location: Option<SessionRepositoryLocation>) -> Self {
        let value = location.as_ref().map_or_else(unavailable, observe_location);
        Self { location, value }
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    /// Check location under the write guard and resample if it changed.
    pub(crate) fn lock_current<'a>(
        mut self,
        db: &'a Mutex<Connection>,
        session_id: &str,
    ) -> Result<(MutexGuard<'a, Connection>, Self), BridgeError> {
        loop {
            let guard = db
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let location = store::repository_location_for_session(&guard, session_id)?;
            if location == self.location {
                return Ok((guard, self));
            }
            drop(guard);
            self = Self::observe(location);
        }
    }
}

fn observe_location(location: &SessionRepositoryLocation) -> Value {
    let started = Instant::now();
    let path = location.resolve();
    let value = path
        .as_deref()
        .map_or_else(unavailable, store::measure_repository_state);
    let worktree = path.map(|path| match path.canonicalize() {
        Ok(canonical) => canonical
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .unwrap_or(&canonical)
            .to_path_buf(),
        Err(_) => path,
    });
    let mut cache = cache();
    if let Some(path) = &worktree {
        let slot = cache.worktrees.entry(path.clone()).or_default();
        // The location could have been invalidated while it was being resolved.
        record_sample(slot, started, &value);
    }
    if let Some(invalidated) = cache
        .cold_refreshes
        .remove(location)
        .filter(|at| *at > started)
    {
        if let Some(path) = &worktree {
            let slot = cache.worktrees.entry(path.clone()).or_default();
            slot.invalidated_at = slot.invalidated_at.max(Some(invalidated));
        }
    }
    cache.locations.insert(location.clone(), worktree);
    value
}

fn record_sample(slot: &mut Slot, started: Instant, value: &Value) {
    if slot
        .stamp
        .as_ref()
        .is_none_or(|(previous, _)| started >= *previous)
    {
        slot.stamp = Some((started, value.clone()));
    }
    if slot.invalidated_at.is_some_and(|at| at <= started) {
        slot.invalidated_at = None;
    }
}

pub(crate) fn for_entry(
    db: &Connection,
    session_id: &str,
    kind: &str,
    prepared: Option<&Value>,
) -> Result<Value, BridgeError> {
    if exact_kind(kind) {
        return Ok(prepared.cloned().unwrap_or_else(unavailable));
    }
    let location = store::repository_location_for_session(db, session_id)?;
    let cache = cache();
    Ok(location
        .as_ref()
        .and_then(|location| cache.locations.get(location))
        .and_then(Option::as_ref)
        .and_then(|path| cache.worktrees.get(path))
        .and_then(|slot| slot.stamp.as_ref())
        .map_or_else(unavailable, |(_, value)| value.clone()))
}

pub(crate) fn invalidate_session(db: &Connection, session_id: &str) {
    let Ok(Some(location)) = store::repository_location_for_session(db, session_id) else {
        return;
    };
    let mut cache = cache();
    let now = Instant::now();
    match cache.locations.get(&location).cloned() {
        Some(Some(path)) => cache.worktrees.entry(path).or_default().invalidated_at = Some(now),
        Some(None) => return,
        None => {
            cache.cold_refreshes.insert(location, now);
        }
    }
    schedule_refresh(&mut cache);
}

/// Editor writes use the same cache invalidation as committed tool events.
pub fn invalidate_containing(path: &Path) {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut cache = cache();
    let now = Instant::now();
    let mut changed = false;
    for (worktree, slot) in &mut cache.worktrees {
        if absolute.starts_with(worktree) {
            slot.invalidated_at = Some(now);
            changed = true;
        }
    }
    if changed {
        schedule_refresh(&mut cache);
    }
}

fn schedule_refresh(cache: &mut Cache) {
    if cache.worker_running {
        REFRESH.notify_one();
    } else if std::thread::Builder::new()
        .name("bridge-repository-stamps".into())
        .spawn(refresh_invalidated)
        .is_ok()
    {
        cache.worker_running = true;
    }
}

fn refresh_invalidated() {
    enum Target {
        Worktree(PathBuf),
        Location(SessionRepositoryLocation),
    }
    loop {
        let mut state = cache();
        let (target, started) = loop {
            let known = state
                .worktrees
                .iter()
                .filter_map(|(path, slot)| slot.invalidated_at.map(|at| (path, at)))
                .min_by_key(|(_, at)| *at)
                .map(|(path, at)| (Target::Worktree(path.clone()), at));
            let cold = state
                .cold_refreshes
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(location, at)| (Target::Location(location.clone()), *at));
            let Some((target, invalidated)) =
                known.into_iter().chain(cold).min_by_key(|(_, at)| *at)
            else {
                state.worker_running = false;
                return;
            };
            let due = invalidated + DEBOUNCE;
            let now = Instant::now();
            if due <= now {
                match &target {
                    Target::Worktree(path) => {
                        if let Some(slot) = state.worktrees.get_mut(path) {
                            slot.invalidated_at = None;
                        }
                    }
                    Target::Location(location) => {
                        state.cold_refreshes.remove(location);
                    }
                }
                break (target, now);
            }
            state = REFRESH
                .wait_timeout(state, due - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        };
        drop(state);
        match target {
            Target::Worktree(path) => {
                let value = store::measure_repository_state(&path);
                record_sample(cache().worktrees.entry(path).or_default(), started, &value);
            }
            Target::Location(location) => {
                observe_location(&location);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |arguments: &[&str]| {
            let output = std::process::Command::new("git")
                .args(arguments)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "git {arguments:?}");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "bridge@example.invalid"]);
        git(&["config", "user.name", "Bridge Test"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("tracked.txt"), "first\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "--quiet", "-m", "initial"]);
        dir
    }

    fn chat(db: &Connection, id: &str, cwd: &Path) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd) VALUES(?1,NULL,'claude','Chat','working','reported',?2)",
            params![id, cwd.to_string_lossy()],
        )
        .unwrap();
    }

    fn git_runs<T>(work: impl FnOnce() -> T) -> (u64, T) {
        let before = crate::git::git_processes_started_on_this_thread();
        let value = work();
        (
            crate::git::git_processes_started_on_this_thread() - before,
            value,
        )
    }

    #[test]
    fn a_cold_mid_turn_append_never_starts_git_under_the_database_guard() {
        let repo = repository();
        let db = Mutex::new(crate::store::open(Path::new(":memory:")).unwrap());
        let guard = db.lock().unwrap();
        chat(&guard, "repo-stamp-cold", repo.path());

        let (runs, stored) = git_runs(|| {
            crate::store::session_event(
                &guard,
                "repo-stamp-cold",
                &crate::agent::NormalizedEvent::new("command.started"),
                &serde_json::Value::Null,
            )
            .unwrap()
        });
        assert_eq!(stored.sequence, 1);
        assert_eq!(runs, 0, "writing a cold mid-turn entry started Git");
    }

    #[test]
    fn boundary_observes_an_external_edit_and_the_write_runs_no_git() {
        let repo = repository();
        let db = Mutex::new(crate::store::open(Path::new(":memory:")).unwrap());
        let session_id = "repo-stamp-boundary";
        chat(&db.lock().unwrap(), session_id, repo.path());
        let append = || {
            let prepared = Prepared::for_session(&db, session_id).unwrap();
            let (guard, prepared) = prepared.lock_current(&db, session_id).unwrap();
            let (runs, result) = git_runs(|| {
                crate::store::session_event_with_repository_state(
                    &guard,
                    session_id,
                    &crate::agent::NormalizedEvent::new("turn.completed"),
                    &Value::Null,
                    Some(prepared.value()),
                )
            });
            result.unwrap();
            assert_eq!(runs, 0);
        };
        append();
        std::fs::write(repo.path().join("tracked.txt"), "external edit").unwrap();
        append();

        let entries = crate::store::session_entries(&db.lock().unwrap(), session_id).unwrap();
        assert_eq!(entries[0].payload["_bridgeRepoState"]["status"], "clean");
        let dirty = &entries[1].payload["_bridgeRepoState"];
        assert_eq!(dirty["status"], "dirty");
        assert_eq!(dirty, &crate::store::repository_state_for_path(repo.path()));
    }

    #[test]
    fn a_prepared_stamp_is_replaced_when_the_session_moves_to_another_worktree() {
        let first = repository();
        let second = repository();
        std::fs::write(second.path().join("tracked.txt"), "only dirty here").unwrap();
        let db = Mutex::new(crate::store::open(Path::new(":memory:")).unwrap());
        let session_id = "repo-stamp-moved";
        chat(&db.lock().unwrap(), session_id, first.path());
        let prepared = Prepared::for_session(&db, session_id).unwrap();
        db.lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET cwd=?2 WHERE id=?1",
                params![session_id, second.path().to_str().unwrap()],
            )
            .unwrap();
        let (_guard, prepared) = prepared.lock_current(&db, session_id).unwrap();
        assert_eq!(prepared.value()["status"], "dirty");
    }

    #[test]
    fn a_committed_file_change_refreshes_even_a_cold_worktree() {
        let repo = repository();
        let db = Mutex::new(crate::store::open(Path::new(":memory:")).unwrap());
        let session_id = "repo-stamp-cold-change";
        chat(&db.lock().unwrap(), session_id, repo.path());
        std::fs::write(repo.path().join("tracked.txt"), "edited").unwrap();
        crate::store::session_event(
            &db.lock().unwrap(),
            session_id,
            &crate::agent::NormalizedEvent::new("file_change.completed"),
            &Value::Null,
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let stamp =
                for_entry(&db.lock().unwrap(), session_id, "command.started", None).unwrap();
            if stamp["status"] == "dirty" {
                break;
            }
            assert!(Instant::now() < deadline, "worktree cache never refreshed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_late_scan_cannot_replace_a_newer_boundary_sample() {
        let started = Instant::now();
        let mut slot = Slot::default();
        let dirty = json!({"status":"dirty"});
        record_sample(&mut slot, started + Duration::from_millis(1), &dirty);
        record_sample(&mut slot, started, &json!({"status":"clean"}));
        assert_eq!(slot.stamp.unwrap().1, dirty);
    }

    #[test]
    fn compaction_writes_one_observation_for_all_three_boundary_rows() {
        use crate::compaction_controller::{
            CheckpointOutcome, CompactionController, CompactionReason,
        };

        let repo = repository();
        std::fs::write(repo.path().join("tracked.txt"), "edited").unwrap();
        let db = Mutex::new(crate::store::open(Path::new(":memory:")).unwrap());
        let session_id = "repo-stamp-compaction";
        chat(&db.lock().unwrap(), session_id, repo.path());
        let prepared = Prepared::for_session(&db, session_id).unwrap();
        let expected = crate::store::repository_state_for_path(repo.path());
        let (guard, prepared) = prepared.lock_current(&db, session_id).unwrap();
        let (runs, outcome) = git_runs(|| {
            CompactionController::reconstruct_with_repository_state(
                &guard,
                session_id,
                "",
                CompactionReason::PhaseBoundary,
                Some(prepared.value()),
            )
        });
        assert!(matches!(
            outcome.unwrap(),
            CheckpointOutcome::Completed { .. }
        ));
        assert_eq!(runs, 0);
        let entries = crate::store::session_entries(&guard, session_id).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.kind.as_str())
                .collect::<Vec<_>>(),
            ["checkpoint", "compaction", "branch.summary"]
        );
        for entry in entries {
            assert_eq!(entry.payload["_bridgeRepoState"], expected);
        }
    }
}
