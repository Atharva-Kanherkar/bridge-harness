use crate::{store, BridgeError};
use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyBaseline {
    pub workspace_path: String,
    pub tracked_status: String,
}

impl ReadOnlyBaseline {
    pub fn capture(workspace_path: &str) -> Result<Self, BridgeError> {
        Ok(Self {
            workspace_path: workspace_path.into(),
            tracked_status: tracked_status(Path::new(workspace_path))?,
        })
    }
}

pub fn tracked_status(path: &Path) -> Result<String, BridgeError> {
    let output = crate::git::git_command(path)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()?;
    if !output.status.success() {
        return Err(BridgeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

pub fn verify_and_record(
    db: &Connection,
    session_id: &str,
    baseline: &ReadOnlyBaseline,
) -> Result<bool, BridgeError> {
    let current = tracked_status(Path::new(&baseline.workspace_path))?;
    record_status_comparison(db, session_id, &baseline.tracked_status, &current)
}

fn record_status_comparison(
    db: &Connection,
    session_id: &str,
    baseline: &str,
    current: &str,
) -> Result<bool, BridgeError> {
    if baseline == current {
        return Ok(false);
    }
    store::event(
        db,
        "sandbox",
        "worker.read_only_violation",
        session_id,
        &format!(
            "Tracked workspace state changed during a read-only worker. Before: {:?}; after: {:?}",
            baseline.trim(),
            current.trim()
        ),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repository() -> tempfile::TempDir {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .unwrap();
            assert!(status.success(), "git command failed: {args:?}");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "bridge-test@example.invalid"]);
        git(&["config", "user.name", "Bridge Test"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("tracked.txt"), "before\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-m", "fixture", "-q"]);
        fixture
    }

    #[test]
    fn tracked_status_ignores_untracked_artifacts_and_reports_tracked_edits() {
        let fixture = repository();
        std::fs::write(fixture.path().join("artifact.log"), "build output").unwrap();
        assert_eq!(tracked_status(fixture.path()).unwrap(), "");
        std::fs::write(fixture.path().join("tracked.txt"), "after\n").unwrap();
        let status = tracked_status(fixture.path()).unwrap();
        assert!(status.contains("tracked.txt"));
        assert!(!status.contains("artifact.log"));
    }

    #[test]
    fn comparison_records_only_changed_tracked_state() {
        let db = crate::store::open(Path::new(":memory:")).unwrap();
        assert!(!record_status_comparison(&db, "worker", "", "").unwrap());
        assert!(record_status_comparison(&db, "worker", "", " M tracked.txt\n").unwrap());
        let (kind, entity): (String, String) = db
            .query_row(
                "SELECT kind,entity_id FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "worker.read_only_violation");
        assert_eq!(entity, "worker");
    }
}
