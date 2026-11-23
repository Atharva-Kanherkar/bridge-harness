//! Load benchmarks for "many chats at once", run by hand against real data.
//!
//! Every test here is `#[ignore]` and skips unless its input is named, so the
//! normal suite never pays for them:
//!
//! ```text
//! BRIDGE_PERF_DB=<copy of bridge.db> \
//! BRIDGE_PERF_REPOS=<git clone>:<git clone>:... \
//! cargo test -p bridge-core --lib perf_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `BRIDGE_PERF_DB` must be a copy (`sqlite3 -readonly live.db ".backup copy.db"`,
//! or a clone of a history snapshot), never the live file. Every write a bench
//! makes to it is rolled back. Nothing here boots a core, so a copy's recorded
//! adapter PIDs are never signalled.
//! `BRIDGE_PERF_REPOS` should be scratch clones: every chat runs `git status`
//! in them, which may refresh their index.

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use crate::store;

fn perf_db() -> Option<PathBuf> {
    std::env::var_os("BRIDGE_PERF_DB").map(PathBuf::from)
}

fn perf_repos() -> Vec<PathBuf> {
    std::env::var("BRIDGE_PERF_REPOS")
        .map(|value| {
            value
                .split(':')
                .filter(|part| !part.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn summary(label: &str, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    let total: f64 = samples.iter().sum();
    println!(
        "{label:<44} n={:<5} p50={:>9.2} p95={:>9.2} max={:>9.2} sum={:>10.1} (ms)",
        samples.len(),
        percentile(&samples, 0.5),
        percentile(&samples, 0.95),
        samples.last().copied().unwrap_or(f64::NAN),
        total,
    );
}

/// Best of `runs` wall-clock timings, so one scheduling hiccup on a loaded
/// machine does not become the number.
fn best_of<T>(runs: usize, mut run: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut last = None;
    for _ in 0..runs {
        let started = Instant::now();
        let value = run();
        best = best.min(ms(started.elapsed()));
        last = Some(value);
    }
    (best, last.expect("at least one run"))
}

/// B1: what one app state read costs on this database: the build under the
/// lock, then the serialization every response pays. `full` is the document
/// the app read before (`state/get_state`, background runs included); `app`
/// is what it reads now (`state/get_app_state`).
#[test]
#[ignore = "benchmark: set BRIDGE_PERF_DB to a copy of bridge.db"]
fn perf_bench_app_state() {
    let Some(path) = perf_db() else { return };
    let db = Connection::open(&path).unwrap();
    type Read = fn(&Connection) -> Result<crate::model::BridgeState, crate::BridgeError>;
    for (label, read) in [
        ("full", store::state_with_background_runs as Read),
        ("app ", store::state as Read),
    ] {
        let (build_ms, state) = best_of(3, || read(&db).unwrap());
        let (encode_ms, bytes) = best_of(3, || serde_json::to_vec(&state).unwrap());
        println!(
            "B1 state ({label})         sessions={:>6} build={build_ms:>8.1} ms  serialize={encode_ms:>8.1} ms  bytes={:>6.1} MB",
            state.sessions.len(),
            bytes.len() as f64 / 1_048_576.0,
        );
    }
}

/// B2: the worker pool's once-a-second terminal-outcome read, which runs under
/// the database lock. Read-only: the bench rolls its transaction back.
#[test]
#[ignore = "benchmark: set BRIDGE_PERF_DB to a copy of bridge.db"]
fn perf_bench_worker_pool_pass() {
    let Some(path) = perf_db() else { return };
    let db = Connection::open(&path).unwrap();
    // DDL is transactional in SQLite: the index exists for the second half of
    // this run only, and the copy is left as it was.
    let transaction = db.unchecked_transaction().unwrap();
    for label in ["without index", "with index   "] {
        if label.starts_with("with ") {
            transaction
                .execute_batch("CREATE INDEX IF NOT EXISTS idx_events_kind_entity ON events(kind, entity_id)")
                .unwrap();
        }
        let (best, reported) = best_of(5, || {
            transaction.execute_batch("SAVEPOINT pass").unwrap();
            let reported =
                crate::worker_pool::WorkerPool::report_terminal_queue_outcomes(&transaction)
                    .unwrap();
            transaction.execute_batch("ROLLBACK TO pass; RELEASE pass").unwrap();
            reported.len()
        });
        println!("B2 worker pool terminal pass, {label}  best={best:>8.2} ms  reported={reported}");
    }
    transaction.rollback().unwrap();
}

/// B4: the app's every-5-seconds workspace poll, minus the Git scan both
/// versions share. Before, the write was followed by the whole app state;
/// now it returns the one workspace row. Rolled back.
#[test]
#[ignore = "benchmark: set BRIDGE_PERF_DB to a copy of bridge.db"]
fn perf_bench_workspace_poll() {
    let Some(path) = perf_db() else { return };
    let db = Connection::open(&path).unwrap();
    let Some(workspace_id): Option<String> = db
        .query_row("SELECT id FROM workspaces ORDER BY created_at DESC LIMIT 1", [], |row| row.get(0))
        .ok()
    else {
        return;
    };
    let transaction = db.unchecked_transaction().unwrap();
    let (before_ms, before_bytes) = best_of(3, || {
        transaction
            .execute("UPDATE workspaces SET dirty_files=dirty_files WHERE id=?1", params![workspace_id])
            .unwrap();
        serde_json::to_vec(&store::state_with_background_runs(&transaction).unwrap()).unwrap().len()
    });
    let (after_ms, after_bytes) = best_of(3, || {
        serde_json::to_vec(&store::workspace(&transaction, &workspace_id).unwrap()).unwrap().len()
    });
    transaction.rollback().unwrap();
    println!(
        "B4 workspace poll, store+encode  whole state={before_ms:>8.1} ms ({:.1} MB)  one workspace={after_ms:>6.2} ms ({before_bytes} -> {after_bytes} bytes)",
        before_bytes as f64 / 1_048_576.0,
    );
}

/// B3: twelve chats streaming at once, each storing entries for a chat in a
/// git repository, all through one shared database lock as in the core. The
/// numbers that matter: how long each entry holds the lock, and how long the
/// others wait for it.
#[test]
#[ignore = "benchmark: set BRIDGE_PERF_REPOS to scratch git clones"]
fn perf_bench_concurrent_chat_streams() {
    let repos = perf_repos();
    if repos.is_empty() {
        return;
    }
    const CHATS: usize = 12;
    const ENTRIES_PER_CHAT: usize = 40;
    let home = tempfile::tempdir().unwrap();
    let db = store::open(&home.path().join("bridge.db")).unwrap();
    let mut sessions = Vec::new();
    for index in 0..CHATS {
        let id = format!("perf-chat-{index}");
        let cwd = repos[index % repos.len()].display().to_string();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd) VALUES(?1,NULL,'claude','Chat','working','reported',?2)",
            params![id, cwd],
        )
        .unwrap();
        sessions.push(id);
    }
    let db = Arc::new(Mutex::new(db));
    let barrier = Arc::new(Barrier::new(CHATS));
    let started = Instant::now();
    let handles: Vec<_> = sessions
        .into_iter()
        .map(|session_id| {
            let db = db.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let mut waits = Vec::new();
                let mut holds = Vec::new();
                let git_before = crate::git::git_processes_started_on_this_thread();
                for index in 0..ENTRIES_PER_CHAT {
                    let event = crate::agent::NormalizedEvent {
                        kind: "command.completed".into(),
                        item_id: Some(format!("item-{index}")),
                        role: Some("tool".into()),
                        status: Some("completed".into()),
                        title: Some("Bash".into()),
                        text: Some("ok".into()),
                        data: serde_json::json!({"command": "true", "exitCode": 0}),
                    };
                    before_entry_lock(&session_id);
                    let requested = Instant::now();
                    let guard = db.lock().unwrap();
                    let acquired = Instant::now();
                    store::session_event(&guard, &session_id, &event, &serde_json::json!({}))
                        .unwrap();
                    drop(guard);
                    waits.push(ms(acquired - requested));
                    holds.push(ms(acquired.elapsed()));
                    // A provider frame arrives every few milliseconds while a
                    // chat streams; do not let one thread monopolize the lock.
                    std::thread::sleep(Duration::from_millis(5));
                }
                let git = crate::git::git_processes_started_on_this_thread() - git_before;
                (waits, holds, git)
            })
        })
        .collect();
    let mut waits = Vec::new();
    let mut holds = Vec::new();
    let mut git = 0;
    for handle in handles {
        let (thread_waits, thread_holds, thread_git) = handle.join().unwrap();
        waits.extend(thread_waits);
        holds.extend(thread_holds);
        git += thread_git;
    }
    let wall = ms(started.elapsed());
    println!(
        "B3 {CHATS} chats x {ENTRIES_PER_CHAT} entries over {} repos: wall={wall:.0} ms, git processes on the entry path={git}",
        repos.len()
    );
    summary("B3 lock held per stored entry", holds);
    summary("B3 wait for the lock per stored entry", waits);
}

/// What the frame reader does before it takes the database lock: any `git`
/// the entry's repository stamp needs runs here, while the lock is free.
fn before_entry_lock(session_id: &str) {
    crate::repo_stamp::refresh_before_lock(session_id);
}

/// B5: SHA-256 throughput as a history snapshot pays it, over a 512 MiB file.
#[test]
#[ignore = "benchmark: set BRIDGE_PERF_HASH=1"]
fn perf_bench_snapshot_hash() {
    if std::env::var_os("BRIDGE_PERF_HASH").is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blob");
    let chunk: Vec<u8> = (0..1_048_576u32).map(|index| (index % 251) as u8).collect();
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&path).unwrap();
        for _ in 0..512 {
            file.write_all(&chunk).unwrap();
        }
    }
    let (best, _) = best_of(2, || store::hash_file_streaming(&path).unwrap());
    println!(
        "B5 snapshot hash 512 MiB   best={best:>8.0} ms  throughput={:>7.1} MB/s",
        512.0 / (best / 1000.0)
    );
}
