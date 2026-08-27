//! Durable launch ledger for provider child processes.
//!
//! Rust destructors do not run when a supervisor is SIGKILLed, crashes, or is
//! an aborted test binary, so any child spawned by that supervisor survives
//! re-parented to PID 1. The session table's adapter claims (migration 12)
//! cover exactly one process per session row; discovery and control servers
//! were never registered anywhere. This module records every such launch in a
//! file per child under `<data_dir>/process-ledger/` the moment it is spawned,
//! removes the file on verified stop, and reaps survivors at the next boot.
//!
//! The reaper fails closed twice over: an entry is only acted on when its
//! recorded supervisor is verifiably gone, and a kill is only issued when the
//! live child's identity (`ps` start time + command) matches the recorded one
//! exactly. PID reuse therefore clears the entry without a kill.

use crate::{adapters, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Where launch records live. A lock rather than a `OnceLock` because
/// [`crate::BridgeCore::boot`] can run more than once in a process and the
/// most recent boot owns the ledger.
static LEDGER_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

const RECORD_SCHEMA_VERSION: u32 = 1;

pub fn register_ledger_root(root: impl Into<PathBuf>) {
    *LEDGER_ROOT
        .write()
        .expect("the ledger root lock is never poisoned") = Some(root.into());
}

fn ledger_root() -> Option<PathBuf> {
    LEDGER_ROOT
        .read()
        .expect("the ledger root lock is never poisoned")
        .clone()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRecord {
    pub schema_version: u32,
    pub pid: u32,
    pub process_identity: String,
    pub supervisor_pid: u32,
    pub supervisor_identity: String,
    pub kind: String,
    pub label: String,
    pub created_at: String,
}

/// Removes its ledger file when dropped — the normal-stop half of the
/// contract. Abnormal supervisor death skips the drop and leaves the file for
/// boot recovery to act on.
#[derive(Default)]
pub struct LaunchGuard {
    path: Option<PathBuf>,
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Record a freshly spawned child in the registered ledger. Best effort by
/// design: an unregistered ledger or an unverifiable identity yields an empty
/// guard rather than an error, because refusing to launch over bookkeeping
/// would be worse than the leak the ledger exists to close.
pub fn record_launch(kind: &str, label: &str, pid: u32) -> LaunchGuard {
    let Some(root) = ledger_root() else {
        return LaunchGuard::default();
    };
    record_launch_in_dir(&root, kind, label, pid)
}

pub fn record_launch_in_dir(root: &Path, kind: &str, label: &str, pid: u32) -> LaunchGuard {
    let Some(process_identity) = adapters::process_identity(pid) else {
        return LaunchGuard::default();
    };
    let supervisor_pid = std::process::id();
    let Some(supervisor_identity) = adapters::process_identity(supervisor_pid) else {
        return LaunchGuard::default();
    };
    let record = LaunchRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        pid,
        process_identity,
        supervisor_pid,
        supervisor_identity,
        kind: kind.to_owned(),
        label: label.to_owned(),
        created_at: Utc::now().to_rfc3339(),
    };
    let Ok(bytes) = serde_json::to_vec_pretty(&record) else {
        return LaunchGuard::default();
    };
    if std::fs::create_dir_all(root).is_err() {
        return LaunchGuard::default();
    }
    let id = uuid::Uuid::new_v4().simple();
    let path = root.join(format!("{pid}-{id}.json"));
    let pending = root.join(format!(".{pid}-{id}.tmp"));
    if std::fs::write(&pending, bytes).is_err() || std::fs::rename(&pending, &path).is_err() {
        let _ = std::fs::remove_file(&pending);
        return LaunchGuard::default();
    }
    LaunchGuard { path: Some(path) }
}

/// Record how long a harness took from child spawn to the readiness boundary
/// it names, so cold-start regressions show up in logs without instrumenting
/// every call site by hand. `boundary` is what was actually awaited — the
/// harnesses do not all observe the same thing, and a number whose boundary is
/// unstated is a number nobody can compare. Stderr, like every other
/// diagnostic this app prints: the workspace installs no tracing subscriber,
/// so a `tracing::info!` here would be discarded before it reached anyone.
pub fn log_spawn_to_ready(harness: &str, boundary: &str, spawned_at: std::time::Instant) {
    eprintln!(
        "bridge: adapter spawn-to-ready harness={harness} boundary={boundary} elapsed_ms={}",
        spawned_at.elapsed().as_millis()
    );
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub killed: usize,
    pub cleared: usize,
}

/// Reap ledgered children whose supervisor is gone. Called at boot, with the
/// explicit directory rather than the process-global registration, before the
/// adapter registry spawns anything new.
pub fn recover_in_dir(db: &Connection, root: &Path) -> Result<RecoveryOutcome, BridgeError> {
    let mut outcome = RecoveryOutcome::default();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(outcome);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let record = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LaunchRecord>(&bytes).ok());
        let Some(record) = record else {
            // Our directory, our schema: anything unreadable is a torn write
            // from a dead supervisor, not evidence of a live child.
            let _ = std::fs::remove_file(&path);
            outcome.cleared += 1;
            continue;
        };
        if record.schema_version != RECORD_SCHEMA_VERSION {
            let _ = std::fs::remove_file(&path);
            outcome.cleared += 1;
            continue;
        }
        // Never touch a child whose recorded supervisor is still the process
        // that recorded it — including ourselves after an in-process reboot.
        if record.supervisor_pid == std::process::id() {
            continue;
        }
        let supervisor_identity = adapters::process_identity(record.supervisor_pid);
        if supervisor_identity.as_deref() == Some(record.supervisor_identity.as_str()) {
            continue;
        }
        // A `None` identity is how a dead supervisor looks — and also how a
        // transient probe failure looks. Only a clean listing that shows the
        // PID gone counts as dead; anything else keeps the entry for the next
        // boot instead of risking a live supervisor's child. A differing
        // identity is the PID verifiably reused, which is the supervisor gone.
        if supervisor_identity.is_none()
            && !process_is_verifiably_gone(record.supervisor_pid)
        {
            continue;
        }
        let live_identity = adapters::process_identity(record.pid);
        if live_identity.as_deref() == Some(record.process_identity.as_str()) {
            if adapters::terminate_process_group(record.pid) {
                record_recovery_event(db, "process.orphan_killed", &record)?;
                let _ = std::fs::remove_file(&path);
                outcome.killed += 1;
            } else {
                // Keep the file so the next boot retries; a stuck process is
                // not a reason to abort this one.
                record_recovery_event(db, "process.orphan_kill_failed", &record)?;
            }
        } else {
            if live_identity.is_some() {
                record_recovery_event(db, "process.orphan_identity_mismatch", &record)?;
            }
            let _ = std::fs::remove_file(&path);
            outcome.cleared += 1;
        }
    }
    Ok(outcome)
}

/// True only when a clean `ps` run lists nothing for the PID. A probe that
/// cannot run at all returns false, so callers treat the process as possibly
/// alive and fail closed.
fn process_is_verifiably_gone(pid: u32) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

fn record_recovery_event(
    db: &Connection,
    kind: &str,
    record: &LaunchRecord,
) -> Result<(), BridgeError> {
    let body = format!(
        "{} pid {} ({}) launched {} by supervisor {}",
        record.kind, record.pid, record.label, record.created_at, record.supervisor_pid
    );
    db.execute(
        "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor',?1,?2,?3,?4)",
        params![
            kind,
            format!("process:{}", record.kind),
            body,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub const LEGACY_SWEEP_DISABLE_ENV: &str = "BRIDGE_SKIP_LEGACY_ORPHAN_SWEEP";

/// Terminate `opencode serve` processes that predate the ledger: exact Bridge
/// argv shape, re-parented to PID 1, and running from a Bridge checkout. This
/// is the one-time remediation for orphans accumulated before this module
/// existed; the ledger and the parent-death watchdog prevent new ones.
#[cfg(unix)]
pub fn sweep_legacy_opencode_orphans(db: &Connection) -> Result<usize, BridgeError> {
    if std::env::var_os(LEGACY_SWEEP_DISABLE_ENV).is_some() {
        return Ok(0);
    }
    let output = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=", "-o", "ppid=", "-o", "command="])
        .stderr(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        return Ok(0);
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let candidates = parse_orphan_candidates(&listing);
    if candidates.is_empty() {
        return Ok(0);
    }
    let cwds = query_working_directories(candidates.iter().map(|(pid, _)| *pid));
    let mut killed = 0;
    for (pid, command) in candidates {
        // No verifiable working directory means no kill.
        let Some(cwd) = cwds.get(&pid) else { continue };
        if !cwd_is_bridge_checkout(cwd) {
            continue;
        }
        if adapters::terminate_process_group(pid) {
            let body = format!("legacy orphan pid {pid} in {cwd}: {command}");
            db.execute(
                "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES('supervisor','process.legacy_orphan_killed','process:opencode.legacy',?1,?2)",
                params![body, Utc::now().to_rfc3339()],
            )?;
            killed += 1;
        }
    }
    Ok(killed)
}

#[cfg(not(unix))]
pub fn sweep_legacy_opencode_orphans(_db: &Connection) -> Result<usize, BridgeError> {
    Ok(0)
}

/// Pairs of (pid, command) for processes re-parented to PID 1 whose command
/// line is exactly the shape Bridge spawns.
fn parse_orphan_candidates(listing: &str) -> Vec<(u32, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid_field, line) = line.split_once(char::is_whitespace)?;
            let (ppid_field, command) = line.trim_start().split_once(char::is_whitespace)?;
            let pid = pid_field.parse::<u32>().ok()?;
            let ppid = ppid_field.parse::<u32>().ok()?;
            let command = command.trim_start();
            (ppid == 1 && legacy_command_matches(command)).then(|| (pid, command.to_owned()))
        })
        .collect()
}

/// Exactly `<path to an executable named opencode> serve --hostname 127.0.0.1
/// --port <digits>` — anything else (extra flags, another hostname, another
/// binary name) is not ours to touch.
fn legacy_command_matches(command: &str) -> bool {
    let Some((executable, rest)) = command.split_once(" serve --hostname 127.0.0.1 --port ")
    else {
        return false;
    };
    let named_opencode = Path::new(executable)
        .file_name()
        .is_some_and(|name| name == "opencode");
    let port_only = !rest.is_empty()
        && rest.len() <= 5
        && rest.bytes().all(|byte| byte.is_ascii_digit());
    named_opencode && port_only
}

fn cwd_is_bridge_checkout(cwd: &str) -> bool {
    Path::new(cwd)
        .components()
        .any(|component| component.as_os_str() == "src-tauri")
}

/// Working directories for the given pids via one batched `lsof` call.
#[cfg(unix)]
fn query_working_directories(
    pids: impl Iterator<Item = u32>,
) -> std::collections::HashMap<u32, String> {
    let list = pids.map(|pid| pid.to_string()).collect::<Vec<_>>().join(",");
    let output = std::process::Command::new("lsof")
        .args(["-a", "-p", &list, "-d", "cwd", "-Fn"])
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        return Default::default();
    };
    parse_lsof_cwds(&String::from_utf8_lossy(&output.stdout))
}

fn parse_lsof_cwds(listing: &str) -> std::collections::HashMap<u32, String> {
    let mut cwds = std::collections::HashMap::new();
    let mut current = None;
    for line in listing.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current = pid.parse::<u32>().ok();
        } else if let Some(path) = line.strip_prefix('n') {
            if let Some(pid) = current {
                cwds.insert(pid, path.to_owned());
            }
        }
    }
    cwds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// Serializes tests that touch the process-global ledger root.
    static LEDGER_ROOT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_db() -> Connection {
        crate::store::open(Path::new(":memory:")).expect("in-memory store opens")
    }

    fn spawn_sleeper() -> std::process::Child {
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::null()).stdout(Stdio::null());
        adapters::configure_process_group(&mut command);
        command.spawn().expect("sleep fixture spawns")
    }

    fn recovery_events(db: &Connection, kind: &str) -> i64 {
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE kind=?1",
            params![kind],
            |row| row.get(0),
        )
        .expect("events count")
    }

    #[test]
    fn record_launch_writes_and_guard_removes() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let mut child = spawn_sleeper();
        let guard = record_launch_in_dir(scratch.path(), "opencode.session", "/tmp/x", child.id());
        let files: Vec<_> = std::fs::read_dir(scratch.path())
            .expect("readable")
            .flatten()
            .collect();
        assert_eq!(files.len(), 1);
        let record: LaunchRecord =
            serde_json::from_slice(&std::fs::read(files[0].path()).expect("record readable"))
                .expect("record parses");
        assert_eq!(record.pid, child.id());
        assert_eq!(record.supervisor_pid, std::process::id());
        assert_eq!(record.kind, "opencode.session");
        assert!(!record.process_identity.is_empty());
        drop(guard);
        assert_eq!(std::fs::read_dir(scratch.path()).expect("readable").count(), 0);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn recover_kills_only_dead_supervisor_identity_matches() {
        let _guard = LEDGER_ROOT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = tempfile::tempdir().expect("tempdir");
        let db = test_db();
        let mut child = spawn_sleeper();
        let ledger = record_launch_in_dir(scratch.path(), "opencode.control", "test", child.id());
        // Rewrite the record's supervisor to a dead identity so the entry
        // reads as abandoned even though this test process is alive.
        let path = std::fs::read_dir(scratch.path())
            .expect("readable")
            .flatten()
            .next()
            .expect("one record")
            .path();
        let mut record: LaunchRecord =
            serde_json::from_slice(&std::fs::read(&path).expect("readable")).expect("parses");
        record.supervisor_pid = u32::MAX - 1;
        record.supervisor_identity = "long gone".into();
        std::fs::write(&path, serde_json::to_vec(&record).expect("serializes")).expect("written");
        std::mem::forget(ledger);

        let outcome = recover_in_dir(&db, scratch.path()).expect("recovery runs");
        assert_eq!(outcome.killed, 1);
        assert_eq!(recovery_events(&db, "process.orphan_killed"), 1);
        assert_eq!(std::fs::read_dir(scratch.path()).expect("readable").count(), 0);
        // A kill outcome means the group is verifiably down, so this returns
        // immediately instead of sitting out the sleep.
        let status = child.wait().expect("the killed child is reapable");
        assert!(!status.success(), "the child was terminated, not finished");
    }

    #[test]
    fn recover_skips_live_supervisor() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let db = test_db();
        // A long-lived stand-in supervisor that is verifiably alive.
        let mut supervisor = spawn_sleeper();
        let mut child = spawn_sleeper();
        let ledger = record_launch_in_dir(scratch.path(), "opencode.session", "test", child.id());
        let path = std::fs::read_dir(scratch.path())
            .expect("readable")
            .flatten()
            .next()
            .expect("one record")
            .path();
        let mut record: LaunchRecord =
            serde_json::from_slice(&std::fs::read(&path).expect("readable")).expect("parses");
        record.supervisor_pid = supervisor.id();
        record.supervisor_identity =
            adapters::process_identity(supervisor.id()).expect("supervisor identity");
        std::fs::write(&path, serde_json::to_vec(&record).expect("serializes")).expect("written");
        std::mem::forget(ledger);

        let outcome = recover_in_dir(&db, scratch.path()).expect("recovery runs");
        assert_eq!(outcome, RecoveryOutcome::default());
        assert!(path.exists(), "a live supervisor's entry stays");
        assert!(
            adapters::process_identity(child.id()).is_some(),
            "a live supervisor's child is never killed"
        );
        let _ = child.kill();
        let _ = child.wait();
        let _ = supervisor.kill();
        let _ = supervisor.wait();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn recover_refuses_identity_mismatch() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let db = test_db();
        let mut child = spawn_sleeper();
        let ledger = record_launch_in_dir(scratch.path(), "opencode.session", "test", child.id());
        let path = std::fs::read_dir(scratch.path())
            .expect("readable")
            .flatten()
            .next()
            .expect("one record")
            .path();
        let mut record: LaunchRecord =
            serde_json::from_slice(&std::fs::read(&path).expect("readable")).expect("parses");
        record.supervisor_pid = u32::MAX - 1;
        record.supervisor_identity = "long gone".into();
        record.process_identity = "someone else entirely".into();
        std::fs::write(&path, serde_json::to_vec(&record).expect("serializes")).expect("written");
        std::mem::forget(ledger);

        let outcome = recover_in_dir(&db, scratch.path()).expect("recovery runs");
        assert_eq!(outcome.killed, 0);
        assert_eq!(outcome.cleared, 1);
        assert_eq!(recovery_events(&db, "process.orphan_identity_mismatch"), 1);
        assert!(
            adapters::process_identity(child.id()).is_some(),
            "an identity mismatch is never killed"
        );
        assert!(!path.exists(), "the stale entry is cleared");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn recover_clears_corrupt_and_versioned_files() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let db = test_db();
        std::fs::write(scratch.path().join("torn.json"), b"{not json").expect("written");
        let stale = LaunchRecord {
            schema_version: RECORD_SCHEMA_VERSION + 1,
            pid: 1,
            process_identity: "x".into(),
            supervisor_pid: u32::MAX - 1,
            supervisor_identity: "gone".into(),
            kind: "opencode.session".into(),
            label: "test".into(),
            created_at: Utc::now().to_rfc3339(),
        };
        std::fs::write(
            scratch.path().join("versioned.json"),
            serde_json::to_vec(&stale).expect("serializes"),
        )
        .expect("written");
        std::fs::write(scratch.path().join("notes.txt"), b"keep me").expect("written");

        let outcome = recover_in_dir(&db, scratch.path()).expect("recovery runs");
        assert_eq!(outcome.cleared, 2);
        assert!(scratch.path().join("notes.txt").exists(), "foreign files stay");
    }

    #[test]
    fn gone_probe_distinguishes_live_dead_and_reaped() {
        assert!(
            !process_is_verifiably_gone(std::process::id()),
            "a live process is never verifiably gone"
        );
        let mut child = spawn_sleeper();
        let pid = child.id();
        assert!(!process_is_verifiably_gone(pid));
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            process_is_verifiably_gone(pid),
            "a killed and reaped process lists nothing"
        );
    }

    #[test]
    fn legacy_matcher_accepts_observed_shapes() {
        assert!(legacy_command_matches(
            "/opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port 54269"
        ));
        assert!(legacy_command_matches(
            "opencode serve --hostname 127.0.0.1 --port 1"
        ));
    }

    #[test]
    fn legacy_matcher_rejects_near_misses() {
        for command in [
            "/opt/homebrew/bin/opencode serve --hostname 0.0.0.0 --port 54269",
            "/opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port 54269 --verbose",
            "/opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port abc",
            "/opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port 123456",
            "/usr/bin/other serve --hostname 127.0.0.1 --port 54269",
            "opencode-imposter serve --hostname 127.0.0.1 --port 54269",
            "opencode run --hostname 127.0.0.1 --port 54269",
        ] {
            assert!(!legacy_command_matches(command), "{command}");
        }
    }

    #[test]
    fn candidates_require_ppid_one() {
        let listing = "\
  100     1 /opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port 54269
  200   583 /opt/homebrew/bin/opencode serve --hostname 127.0.0.1 --port 54270
  300     1 /usr/bin/vim notes.txt
";
        let candidates = parse_orphan_candidates(listing);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, 100);
    }

    #[test]
    fn checkout_matcher_requires_src_tauri_component() {
        assert!(cwd_is_bridge_checkout("/Users/x/harness/src-tauri"));
        assert!(cwd_is_bridge_checkout(
            "/Users/x/harness/src-tauri/bridge-core"
        ));
        assert!(!cwd_is_bridge_checkout("/Users/x/other-project"));
        assert!(!cwd_is_bridge_checkout("/Users/x/src-tauri-lookalike"));
    }

    #[test]
    fn lsof_output_parses_into_pairs() {
        let listing = "p100\nfcwd\nn/Users/x/harness/src-tauri\np200\nfcwd\nn/tmp\n";
        let cwds = parse_lsof_cwds(listing);
        assert_eq!(cwds.get(&100).map(String::as_str), Some("/Users/x/harness/src-tauri"));
        assert_eq!(cwds.get(&200).map(String::as_str), Some("/tmp"));
    }

    #[test]
    fn global_root_registration_round_trips() {
        let _guard = LEDGER_ROOT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Booting fixtures elsewhere register the ledger root too; hold the
        // shared boot lock so this test's root cannot be swapped mid-flight.
        let _boot_guard = crate::managed_runtime::MANAGED_ROOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let scratch = tempfile::tempdir().expect("tempdir");
        register_ledger_root(scratch.path());
        let mut child = spawn_sleeper();
        let guard = record_launch("opencode.session", "test", child.id());
        assert_eq!(std::fs::read_dir(scratch.path()).expect("readable").count(), 1);
        drop(guard);
        assert_eq!(std::fs::read_dir(scratch.path()).expect("readable").count(), 0);
        let _ = child.kill();
        let _ = child.wait();
    }
}
