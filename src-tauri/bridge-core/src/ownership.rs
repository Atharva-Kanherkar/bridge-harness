//! The single-owner invariant for a Bridge data directory.
//!
//! Exactly one process may own a data directory's sessions, SQLite stores,
//! PTYs, and provider processes at a time — a second owner would race turn
//! commits and orphan adapters. Ownership is an advisory OS file lock on
//! `<data_dir>/owner.lock`, so it cannot outlive its process: a `kill -9`
//! releases it with no stale-lockfile recovery dance, and the next owner's
//! boot-time recovery handles whatever the dead owner left behind.
//!
//! Both hosts acquire it before [`crate::BridgeCore::boot`]: the `bridged`
//! daemon as [`OwnerKind::Daemon`], the Tauri shell (embedded mode, allowed
//! during migration) as [`OwnerKind::Embedded`]. The lock file carries the
//! owner's identity as JSON so the losing process can say *who* owns the
//! directory instead of a bare "resource busy".

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const LOCK_FILE_NAME: &str = "owner.lock";

/// Which kind of process owns the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    /// The `bridged` daemon.
    Daemon,
    /// The desktop app hosting the runtime in-process.
    Embedded,
}

impl OwnerKind {
    fn describe(self) -> &'static str {
        match self {
            OwnerKind::Daemon => "a bridged daemon",
            OwnerKind::Embedded => "the Bridge desktop app (embedded mode)",
        }
    }
}

/// The identity a lock holder records for the losing process's error message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerIdentity {
    pub kind: OwnerKind,
    pub pid: u32,
    pub version: String,
    pub started_at: String,
}

/// Why the lease could not be acquired.
#[derive(Debug)]
pub enum OwnershipError {
    /// Another live process holds the lock. Identity is `None` when the
    /// holder's metadata could not be read (mid-write or a foreign locker).
    Held {
        data_dir: PathBuf,
        holder: Option<OwnerIdentity>,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnershipError::Held { data_dir, holder } => match holder {
                Some(holder) => write!(
                    formatter,
                    "{} (pid {}, version {}, started {}) already owns the Bridge data directory {} — \
                     stop it first, or point this process at a different --data-dir",
                    holder.kind.describe(),
                    holder.pid,
                    holder.version,
                    holder.started_at,
                    data_dir.display(),
                ),
                None => write!(
                    formatter,
                    "another process already owns the Bridge data directory {} — \
                     stop it first, or point this process at a different --data-dir",
                    data_dir.display(),
                ),
            },
            OwnershipError::Io(error) => {
                write!(formatter, "could not acquire the data-directory lock: {error}")
            }
        }
    }
}

impl std::error::Error for OwnershipError {}

impl From<std::io::Error> for OwnershipError {
    fn from(error: std::io::Error) -> Self {
        OwnershipError::Io(error)
    }
}

/// An exclusive lease on a data directory, held for the life of the value.
/// Dropping it (or dying with it) releases the OS lock.
#[derive(Debug)]
pub struct DataDirLease {
    file: File,
    data_dir: PathBuf,
    identity: OwnerIdentity,
}

impl DataDirLease {
    /// Acquire the exclusive lease, creating the data directory if needed.
    /// Fails fast when a live owner holds the lock — this never waits for an
    /// identified owner to exit. The one bounded wait is for an *identityless*
    /// hold: a lock with no readable identity is transitional (an owner
    /// mid-first-write, or a just-dropped lease whose file handle is still
    /// closing), so an immediate successor — a daemon restart — retries
    /// briefly instead of failing on the release race.
    pub fn acquire(data_dir: &Path, kind: OwnerKind) -> Result<DataDirLease, OwnershipError> {
        std::fs::create_dir_all(data_dir)?;
        let lock_path = data_dir.join(LOCK_FILE_NAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        let transitional_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    // Read the holder's identity from the still-locked file:
                    // OS file locks gate locking, not reading.
                    let mut contents = String::new();
                    file.seek(SeekFrom::Start(0))?;
                    let holder = file
                        .read_to_string(&mut contents)
                        .ok()
                        .and_then(|_| serde_json::from_str::<OwnerIdentity>(&contents).ok());
                    if holder.is_none() && std::time::Instant::now() < transitional_deadline {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        continue;
                    }
                    return Err(OwnershipError::Held {
                        data_dir: data_dir.to_path_buf(),
                        holder,
                    });
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(OwnershipError::Io(error)),
            }
        }
        let identity = OwnerIdentity {
            kind,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").into(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serde_json::to_string_pretty(&identity).unwrap().as_bytes())?;
        file.flush()?;
        Ok(DataDirLease {
            file,
            data_dir: data_dir.to_path_buf(),
            identity,
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn identity(&self) -> &OwnerIdentity {
        &self.identity
    }
}

impl Drop for DataDirLease {
    fn drop(&mut self) {
        // Best-effort: blank the identity so a later reader of an *unlocked*
        // file does not mistake stale metadata for a live owner. The lock
        // itself is released by the OS when the file closes (or the process
        // dies), so correctness never depends on this write.
        let _ = self.file.set_len(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_owner_is_refused_with_the_holders_identity() {
        let fixture = tempfile::tempdir().unwrap();
        let lease = DataDirLease::acquire(fixture.path(), OwnerKind::Daemon).unwrap();
        assert_eq!(lease.identity().pid, std::process::id());

        let refused = DataDirLease::acquire(fixture.path(), OwnerKind::Embedded).unwrap_err();
        let message = refused.to_string();
        assert!(message.contains("a bridged daemon"), "{message}");
        assert!(
            message.contains(&std::process::id().to_string()),
            "{message}"
        );
        assert!(
            message.contains(&fixture.path().display().to_string()),
            "{message}"
        );
        assert!(
            message.contains("--data-dir"),
            "the error must say what to do: {message}"
        );
    }

    #[test]
    fn embedded_and_daemon_owners_exclude_each_other_symmetrically() {
        let fixture = tempfile::tempdir().unwrap();
        let embedded = DataDirLease::acquire(fixture.path(), OwnerKind::Embedded).unwrap();
        let refused = DataDirLease::acquire(fixture.path(), OwnerKind::Daemon).unwrap_err();
        assert!(refused.to_string().contains("desktop app"), "{refused}");
        drop(embedded);
        DataDirLease::acquire(fixture.path(), OwnerKind::Daemon).unwrap();
    }

    #[test]
    fn dropping_the_lease_releases_the_directory() {
        let fixture = tempfile::tempdir().unwrap();
        drop(DataDirLease::acquire(fixture.path(), OwnerKind::Daemon).unwrap());
        // Reacquire twice to prove release is repeatable, not a one-shot.
        drop(DataDirLease::acquire(fixture.path(), OwnerKind::Embedded).unwrap());
        DataDirLease::acquire(fixture.path(), OwnerKind::Daemon).unwrap();
    }

    #[test]
    fn the_lease_creates_a_missing_data_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let nested = fixture.path().join("data/bridge");
        let lease = DataDirLease::acquire(&nested, OwnerKind::Daemon).unwrap();
        assert!(nested.join(LOCK_FILE_NAME).is_file());
        assert_eq!(lease.data_dir(), nested.as_path());
    }

    #[test]
    fn a_dead_owners_lock_is_reclaimed_without_recovery_steps() {
        // Simulate kill -9: the child acquires the lease and is SIGKILLed
        // while holding it; the OS releases the lock with the process, so the
        // next acquire succeeds immediately.
        let fixture = tempfile::tempdir().unwrap();
        let helper_dir = fixture.path().to_path_buf();
        // A child *process* is required — file locks are process-scoped, so a
        // thread would share ours. Re-exec the test binary in helper mode.
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args([
                "--nocapture",
                "--exact",
                "ownership::tests::helper_hold_lock_forever",
            ])
            .env("BRIDGE_LOCK_HELPER_DIR", &helper_dir)
            .env("BRIDGE_LOCK_HELPER", "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Wait until the child signals it holds the lock.
        let marker = helper_dir.join("helper-holds-lock");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "helper never took the lock"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(matches!(
            DataDirLease::acquire(&helper_dir, OwnerKind::Daemon),
            Err(OwnershipError::Held { .. })
        ));
        child.kill().unwrap();
        child.wait().unwrap();
        DataDirLease::acquire(&helper_dir, OwnerKind::Daemon)
            .expect("a SIGKILLed owner's lock must be reclaimable immediately");
    }

    /// Not a test of its own: the re-exec target for the kill -9 test above.
    /// Without the env marker it exits immediately.
    #[test]
    fn helper_hold_lock_forever() {
        if std::env::var_os("BRIDGE_LOCK_HELPER").is_none() {
            return;
        }
        let dir = PathBuf::from(std::env::var_os("BRIDGE_LOCK_HELPER_DIR").unwrap());
        let _lease = DataDirLease::acquire(&dir, OwnerKind::Embedded).unwrap();
        std::fs::write(dir.join("helper-holds-lock"), b"held").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}
