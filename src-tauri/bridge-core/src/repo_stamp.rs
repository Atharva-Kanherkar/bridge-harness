//! Repository state stamps for stored entries, taken off the database lock.
//!
//! Every stored transcript entry records the repository's HEAD and a hash of
//! its dirty files (`_bridgeRepoState`), so a later read can tell whether the
//! folder still matches the point in the conversation the person selected.
//! Taking that stamp means two `git` processes, and entries are stored under
//! the core's one database lock. With ten chats streaming, one chat's
//! `git status` held up every other chat's frames, and the app's reads behind
//! them.
//!
//! The stamp now comes from a per-repository cache:
//!
//! - The frame reader calls [`refresh_before_lock`] before it takes the
//!   database lock. When the repository's stamp is older than [`FRESH_FOR`],
//!   or a tool has reported changing the folder since it was taken, that
//!   thread runs `git` itself, while the lock is free. One refresh per
//!   repository runs at a time; another chat in the same repository keeps the
//!   stamp it has.
//! - Under the lock, [`for_entry`] reuses the cached stamp when it still
//!   describes the folder. An entry a person returns to ([`exact_kind`]: a
//!   message, the end of a turn) reuses it only when it is younger than
//!   [`FRESH_FOR`] and no tool has completed in that repository since; any
//!   other entry reuses a stamp younger than [`USABLE_FOR`]. Otherwise the
//!   stamp is measured there, under the lock, as before.
//! - Storing an entry that reports a tool's effect ([`changes_folder`]) marks
//!   the repository's stamp out of date, so the next frame measures again.
//!
//! So a stream's busy middle (command output, tool starts, status changes)
//! may carry a stamp up to [`USABLE_FOR`] old, and messages carry one taken
//! after the last tool that ran. A change made outside any tool (an editor
//! outside Bridge) less than [`FRESH_FOR`] before a message can be missed.
//! The stamp only feeds the "folder changed since this point" hint; it never
//! gates a write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::Value;

use crate::BridgeError;

/// A stamp younger than this is not refreshed before an entry is stored.
pub const FRESH_FOR: Duration = Duration::from_secs(1);

/// Under the database lock, a stamp younger than this serves entries that
/// nobody returns to.
pub const USABLE_FOR: Duration = Duration::from_secs(5);

/// Sessions remembered at most; past it the map starts over. A forgotten
/// session only loses its reader-side refresh until its next stored entry.
const MAX_SESSIONS: usize = 4096;

#[derive(Default)]
struct Slot {
    stamp: Option<Stamp>,
    /// Bumped whenever something reports changing the folder.
    generation: u64,
    refreshing: bool,
}

struct Stamp {
    value: Value,
    taken: Instant,
    /// The slot's generation when the measurement started.
    generation: u64,
}

impl Slot {
    fn reusable(&self, kind: &str) -> Option<Value> {
        let stamp = self.stamp.as_ref()?;
        let age = stamp.taken.elapsed();
        let usable = if exact_kind(kind) {
            stamp.generation == self.generation && age < FRESH_FOR
        } else {
            age < USABLE_FOR
        };
        usable.then(|| stamp.value.clone())
    }

    fn needs_refresh(&self) -> bool {
        !self.refreshing
            && self.stamp.as_ref().is_none_or(|stamp| {
                stamp.generation != self.generation || stamp.taken.elapsed() >= FRESH_FOR
            })
    }
}

#[derive(Default)]
struct Stamps {
    repositories: HashMap<PathBuf, Slot>,
    /// Where each session's entries are stamped, as last resolved under the
    /// database lock. The reader needs it before it may take that lock.
    sessions: HashMap<String, Option<PathBuf>>,
}

fn lock() -> std::sync::MutexGuard<'static, Stamps> {
    static STAMPS: OnceLock<Mutex<Stamps>> = OnceLock::new();
    STAMPS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Entries a person returns to: their own message, a reply, a turn's end.
/// Rewind and fork start from these, and a finished turn leaves its head on one.
pub fn exact_kind(kind: &str) -> bool {
    matches!(
        kind,
        "user.message" | "assistant.message" | "turn.completed" | "turn.failed"
    )
}

/// Entries that report a tool having run against the folder.
pub fn changes_folder(kind: &str) -> bool {
    matches!(
        kind,
        "command.completed"
            | "file_change.completed"
            | "tool.completed"
            | "diff.updated"
            | "subagent.completed"
    )
}

fn generation(path: &Path) -> u64 {
    lock().repositories.get(path).map_or(0, |slot| slot.generation)
}

fn store(path: &Path, value: &Value, generation: u64) {
    let mut stamps = lock();
    let slot = stamps.repositories.entry(path.to_path_buf()).or_default();
    slot.stamp = Some(Stamp {
        value: value.clone(),
        taken: Instant::now(),
        generation,
    });
}

/// Measure `path` now and keep the result for the entries that follow.
pub fn measure(path: &Path) -> Value {
    let generation = generation(path);
    let value = crate::store::measure_repository_state(path);
    store(path, &value, generation);
    value
}

/// Mark every repository containing `path` as changed, for Bridge's own
/// writes into a folder (an editor save) that no tool event reports.
pub fn invalidate_containing(path: &Path) {
    for (repository, slot) in lock().repositories.iter_mut() {
        if path.starts_with(repository) {
            slot.generation += 1;
        }
    }
}

/// Bring the stamp for `session_id`'s repository up to date while the caller
/// holds no database lock. A no-op until the session's first entry has told
/// the cache where it runs, and while another thread refreshes the same
/// repository.
pub fn refresh_before_lock(session_id: &str) {
    let (path, generation) = {
        let mut stamps = lock();
        let Some(Some(path)) = stamps.sessions.get(session_id).cloned() else {
            return;
        };
        let slot = stamps.repositories.entry(path.clone()).or_default();
        if !slot.needs_refresh() {
            return;
        }
        slot.refreshing = true;
        (path, slot.generation)
    };
    let value = crate::store::measure_repository_state(&path);
    store(&path, &value, generation);
    if let Some(slot) = lock().repositories.get_mut(&path) {
        slot.refreshing = false;
    }
}

/// The stamp to store with a new `kind` entry, called under the database lock.
pub fn for_entry(db: &Connection, session_id: &str, kind: &str) -> Result<Value, BridgeError> {
    let path = crate::store::repository_path_for_session(db, session_id)?;
    let reused = {
        let mut stamps = lock();
        if stamps.sessions.len() >= MAX_SESSIONS && !stamps.sessions.contains_key(session_id) {
            stamps.sessions.clear();
        }
        stamps.sessions.insert(session_id.to_owned(), path.clone());
        let Some(path) = &path else {
            return Ok(serde_json::json!({"status":"unavailable"}));
        };
        stamps.repositories.get(path).and_then(|slot| slot.reusable(kind))
    };
    let path = path.expect("returned above when absent");
    let value = reused.unwrap_or_else(|| measure(&path));
    if changes_folder(kind) {
        if let Some(slot) = lock().repositories.get_mut(&path) {
            slot.generation += 1;
        }
    }
    Ok(value)
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
        (crate::git::git_processes_started_on_this_thread() - before, value)
    }

    fn age(path: &Path, by: Duration) {
        let mut stamps = lock();
        let stamp = stamps.repositories.get_mut(path).unwrap().stamp.as_mut().unwrap();
        stamp.taken = stamp.taken.checked_sub(by).unwrap();
    }

    #[test]
    fn a_streaming_entry_reuses_the_stamp_and_runs_no_git_under_the_lock() {
        let repo = repository();
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        chat(&db, "repo-stamp-stream", repo.path());

        let (runs, first) = git_runs(|| for_entry(&db, "repo-stamp-stream", "user.message").unwrap());
        assert_eq!(first["status"], "clean");
        assert_eq!(runs, 2, "a person's message is measured when it is stored");

        // Output, tool starts and status changes in the middle of a turn: no git.
        for kind in ["turn.started", "command.started", "command.output_delta", "session.status"] {
            let (runs, stamp) = git_runs(|| for_entry(&db, "repo-stamp-stream", kind).unwrap());
            assert_eq!(runs, 0, "{kind} ran git under the lock");
            assert_eq!(stamp, first);
        }
    }

    #[test]
    fn a_tool_completion_makes_the_next_message_see_the_change() {
        let repo = repository();
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        chat(&db, "repo-stamp-tool", repo.path());
        let clean = for_entry(&db, "repo-stamp-tool", "user.message").unwrap();

        // The tool edits the folder, and its completion is stored.
        std::fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();
        let (runs, _) = git_runs(|| for_entry(&db, "repo-stamp-tool", "command.completed").unwrap());
        assert_eq!(runs, 0);

        // The reader measures before it takes the lock, so the reply stored
        // under the lock reuses that measurement.
        let (outside, ()) = git_runs(|| refresh_before_lock("repo-stamp-tool"));
        assert_eq!(outside, 2, "the refresh ran git, outside the lock");
        let (under, reply) = git_runs(|| for_entry(&db, "repo-stamp-tool", "assistant.message").unwrap());
        assert_eq!(under, 0, "no git under the lock");
        assert_eq!(reply["status"], "dirty");
        assert_ne!(reply, clean);
    }

    #[test]
    fn without_a_refresh_a_message_after_a_tool_is_measured_under_the_lock() {
        let repo = repository();
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        chat(&db, "repo-stamp-nofresh", repo.path());
        for_entry(&db, "repo-stamp-nofresh", "user.message").unwrap();
        std::fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();
        for_entry(&db, "repo-stamp-nofresh", "file_change.completed").unwrap();
        let (runs, reply) = git_runs(|| for_entry(&db, "repo-stamp-nofresh", "assistant.message").unwrap());
        assert_eq!(runs, 2);
        assert_eq!(reply["status"], "dirty");
    }

    #[test]
    fn the_reader_refreshes_only_a_stale_or_changed_stamp() {
        let repo = repository();
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        // Unknown session: nothing to refresh, and no git.
        assert_eq!(git_runs(|| refresh_before_lock("repo-stamp-unknown")).0, 0);
        chat(&db, "repo-stamp-age", repo.path());
        for_entry(&db, "repo-stamp-age", "user.message").unwrap();
        assert_eq!(git_runs(|| refresh_before_lock("repo-stamp-age")).0, 0, "fresh");
        age(repo.path(), FRESH_FOR);
        assert_eq!(git_runs(|| refresh_before_lock("repo-stamp-age")).0, 2, "a second old");
        assert_eq!(git_runs(|| refresh_before_lock("repo-stamp-age")).0, 0, "fresh again");
        // Past USABLE_FOR even a streaming entry measures under the lock.
        age(repo.path(), USABLE_FOR);
        assert_eq!(git_runs(|| for_entry(&db, "repo-stamp-age", "command.started").unwrap()).0, 2);
    }

    #[test]
    fn an_editor_save_invalidates_the_repository_that_holds_the_file() {
        let repo = repository();
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        chat(&db, "repo-stamp-save", repo.path());
        for_entry(&db, "repo-stamp-save", "user.message").unwrap();
        std::fs::write(repo.path().join("tracked.txt"), "saved\n").unwrap();
        invalidate_containing(&repo.path().join("tracked.txt"));
        let (runs, reply) = git_runs(|| for_entry(&db, "repo-stamp-save", "assistant.message").unwrap());
        assert_eq!(runs, 2);
        assert_eq!(reply["status"], "dirty");
    }

    #[test]
    fn a_chat_outside_any_repository_is_unavailable_without_git() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('repo-stamp-none',NULL,'claude','Chat','working','reported')",
            [],
        )
        .unwrap();
        let (runs, stamp) = git_runs(|| for_entry(&db, "repo-stamp-none", "assistant.message").unwrap());
        assert_eq!(runs, 0);
        assert_eq!(stamp, serde_json::json!({"status":"unavailable"}));
        assert_eq!(git_runs(|| refresh_before_lock("repo-stamp-none")).0, 0);
    }
}
