//! The end-to-end proof epic #161 requires, for the half a machine can run.
//!
//! Every unit test in this epic passes against fixtures. None of them has ever
//! fetched a real vendor closure, so nothing has yet proven that a 270 MB npm
//! tree survives the digest walk, the shim pruning, the atomic promotion, and the
//! receipt round trip. That is where the remaining bugs live: the same pattern
//! bit the delegation epic, where every unit test passed while every live stage
//! was broken.
//!
//! Opt-in because it needs the network, `npm`, and several minutes:
//!
//! ```text
//! cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core \
//!   --test managed_agents_live -- --ignored --nocapture
//! ```
//!
//! `--test-threads=1` is no longer required. The managed-root registration is
//! process-wide, so two of these running concurrently pointed one test's storage
//! at the other's temp directory — which failed as a bare `No such file or
//! directory` from inside an install, an error that says nothing about its cause.
//! [`exclusive_managed_root`] serializes them instead of relying on the caller
//! remembering a flag.
//!
//! What this covers of the epic's proof: step 1 (start from no managed payload),
//! step 2's mechanics (install — through the API the desktop UI calls, not
//! through the UI), step 5 (uninstall), step 6's file-level half (a runtime the
//! user owns is untouched), and step 7 (reinstall). Steps 3, 4, and the
//! history-readable half of 6 need vendor auth and a real session, so they stay a
//! human pass.

use bridge_core::managed_agents::{self, ManagedAgentError};
use bridge_core::managed_payload::{ManagedPayloadStatus, ManagedPayloadStore};
use bridge_core::managed_runtime;

/// Serializes the tests in this file, which share one process-wide managed root.
static MANAGED_ROOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Register `root` and hold exclusive use of the registration.
///
/// The returned guard must live for the whole test: dropping it early would let a
/// sibling re-register while this test is still reading its own storage. Poison is
/// tolerated because a panicking sibling has already failed the run, and turning
/// that into a second confusing failure here would only obscure the first.
fn exclusive_managed_root(root: std::path::PathBuf) -> std::sync::MutexGuard<'static, ()> {
    let guard = MANAGED_ROOT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    managed_runtime::register_managed_root(root);
    guard
}
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A sessions table with just the columns the liveness check reads.
fn empty_sessions_db() -> Connection {
    let db = Connection::open_in_memory().expect("in-memory db");
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, harness TEXT, adapter_pid INTEGER, \
         adapter_process_identity TEXT);",
    )
    .expect("sessions schema");
    db
}

fn status_of(agent_id: &str) -> ManagedPayloadStatus {
    let root = managed_runtime::managed_root().expect("a managed root is registered");
    ManagedPayloadStore::new(root)
        .status(agent_id)
        .expect("status is readable")
}

fn tree_size(path: &Path) -> (u64, usize) {
    fn walk(path: &Path, bytes: &mut u64, files: &mut usize) {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() {
                walk(&entry.path(), bytes, files);
            } else {
                *bytes += metadata.len();
                *files += 1;
            }
        }
    }
    let mut bytes = 0;
    let mut files = 0;
    walk(path, &mut bytes, &mut files);
    (bytes, files)
}

/// Install → verify → uninstall → verify → reinstall, against the real vendor
/// registry, for one agent.
fn prove(agent_id: &str, user_owned: &Path) {
    let db = empty_sessions_db();
    println!("\n=== {agent_id} ===");

    // Step 1: nothing of Bridge's yet, and the user's own copy is what resolves.
    assert!(
        matches!(status_of(agent_id), ManagedPayloadStatus::NotInstalled),
        "{agent_id} must start with no managed payload"
    );
    let before = std::fs::read(user_owned).ok();

    // Step 2: install through the same API the desktop UI calls.
    let started = Instant::now();
    let installed = managed_agents::install_managed_agent(agent_id)
        .unwrap_or_else(|error| panic!("{agent_id} install failed: {error}"));
    let install_time = started.elapsed();
    println!("  install: {:?} -> {:?}", install_time, installed.outcome);

    let ManagedPayloadStatus::Installed { receipt, entrypoint } = status_of(agent_id) else {
        panic!("{agent_id} is not installed after a successful install");
    };
    let root = managed_runtime::managed_root().unwrap();
    let installation = root.join(&receipt.owned_paths[0]);
    let (bytes, files) = tree_size(&installation);
    println!(
        "  payload: {} MB across {files} files",
        bytes / (1024 * 1024)
    );
    println!("  version: {} platform: {}", receipt.version, receipt.platform);
    println!("  entrypoint: {}", entrypoint.display());

    // The entrypoint the receipt names has to be a real, executable file — this is
    // the claim that only a live tree can falsify, because a fixture always has
    // exactly the shape the test author imagined.
    assert!(entrypoint.is_file(), "{agent_id} entrypoint is not a file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&entrypoint).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "{agent_id} entrypoint is not executable");
    }
    // No symlink may survive into a payload the engine claims to own.
    assert_eq!(
        symlinks_under(&installation),
        Vec::<PathBuf>::new(),
        "{agent_id} payload still contains symlinks"
    );

    // Status must agree with itself on a second read: the digest is recomputed
    // from disk, so a tree that hashes differently the second time would show up
    // here and nowhere else.
    assert!(
        matches!(status_of(agent_id), ManagedPayloadStatus::Installed { .. }),
        "{agent_id} status is not stable across reads"
    );

    // Installing again converges instead of redoing the work.
    let again = managed_agents::install_managed_agent(agent_id).expect("second install");
    println!("  reinstall-when-current: {:?}", again.outcome);

    // Step 5: uninstall.
    let removed = managed_agents::uninstall_managed_agent(&db, agent_id)
        .unwrap_or_else(|error| panic!("{agent_id} uninstall failed: {error}"));
    println!("  uninstall: {:?}", removed.outcome);
    assert!(
        matches!(status_of(agent_id), ManagedPayloadStatus::NotInstalled),
        "{agent_id} still reports installed after uninstall"
    );
    assert!(!installation.exists(), "{agent_id} payload survived uninstall");

    // Step 6, the half a machine can check: the user's own runtime is untouched.
    assert_eq!(
        std::fs::read(user_owned).ok(),
        before,
        "{agent_id} uninstall modified the user's own runtime at {}",
        user_owned.display()
    );

    // Step 7: reinstall works.
    let reinstalled = managed_agents::install_managed_agent(agent_id)
        .unwrap_or_else(|error| panic!("{agent_id} reinstall failed: {error}"));
    println!("  reinstall-after-removal: {:?}", reinstalled.outcome);
    assert!(matches!(
        status_of(agent_id),
        ManagedPayloadStatus::Installed { .. }
    ));

    // Leave the root clean for the next agent.
    managed_agents::uninstall_managed_agent(&db, agent_id).expect("final cleanup");
}

fn symlinks_under(path: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    fn walk(path: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else { continue };
            if metadata.file_type().is_symlink() {
                found.push(entry.path());
            } else if metadata.is_dir() {
                walk(&entry.path(), found);
            }
        }
    }
    walk(path, &mut found);
    found
}

#[test]
#[ignore = "fetches real vendor closures from npm; needs network, npm, and several minutes"]
fn the_full_lifecycle_holds_against_real_vendor_payloads() {
    let fixture = tempfile::tempdir().expect("temp managed root");
    let _root = exclusive_managed_root(fixture.path().join("managed-runtimes"));

    // A stand-in for a runtime the user installed themselves, so the untouched
    // assertion is checked against a real file rather than assumed.
    let user_owned = fixture.path().join("user-owned-runtime");
    std::fs::write(&user_owned, b"the user's own binary").expect("write user runtime");

    let recipes = managed_runtime::builtin_recipes();
    assert_eq!(recipes.len(), 3, "all three agents must have a recipe here");
    for (agent_id, _) in recipes {
        prove(agent_id, &user_owned);
    }

    println!("\nall three agents completed install -> uninstall -> reinstall");
}

#[test]
#[ignore = "fetches a real vendor closure from npm; needs network, npm, and several minutes"]
fn a_live_process_blocks_removal_of_a_real_payload() {
    // The safety rule that matters most, against a real installed tree rather
    // than a fixture: a payload must not be deletable while something is running
    // against it.
    let fixture = tempfile::tempdir().expect("temp managed root");
    let _root = exclusive_managed_root(fixture.path().join("managed-runtimes"));

    // OpenCode is the smallest of the three closures.
    let agent_id = "opencode";
    managed_agents::install_managed_agent(agent_id).expect("install for the busy check");
    let ManagedPayloadStatus::Installed { receipt, .. } = status_of(agent_id) else {
        panic!("expected an installed payload");
    };
    let installation = managed_runtime::managed_root()
        .unwrap()
        .join(&receipt.owned_paths[0]);

    // This test process is a genuinely live pid with a verifiable identity.
    let db = empty_sessions_db();
    let pid = std::process::id();
    db.execute(
        "INSERT INTO sessions(id,harness,adapter_pid,adapter_process_identity) VALUES('s1',?1,?2,?3)",
        rusqlite::params![
            agent_id,
            i64::from(pid),
            bridge_core::adapters::process_identity(pid)
        ],
    )
    .expect("insert live session");

    let refused = managed_agents::uninstall_managed_agent(&db, agent_id)
        .expect_err("removal must be refused while a process is live");
    assert!(
        matches!(refused, ManagedAgentError::Busy { .. }),
        "expected a busy refusal, got {refused}"
    );
    assert!(
        installation.is_dir(),
        "the payload must survive a refused removal"
    );
    println!("live process refused removal: {refused}");

    // With the session gone, removal proceeds.
    db.execute("DELETE FROM sessions", []).expect("clear sessions");
    managed_agents::uninstall_managed_agent(&db, agent_id).expect("removal after the process ends");
    assert!(!installation.exists());
}
