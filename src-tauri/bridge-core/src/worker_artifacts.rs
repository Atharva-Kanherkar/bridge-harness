//! Durable storage for artifacts a read-only worker was authorized to produce.
//!
//! A read-only worker runs inside a transient sandbox whose output directory is
//! deleted once the worker finishes. That is right for the projected runtime
//! configuration it also holds, but wrong for the report or JSON artifact the
//! delegation contract asked the worker to create: the parent was told a path
//! that no longer existed by the time it read the result. This module copies the
//! declared artifacts into the session's durable store first and returns stable
//! references the parent can open after delivery and after a reload.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

/// Per-file ceiling. A worker artifact is a report or small data file, not a
/// build output; anything larger is left in the transient tree rather than
/// filling the data directory.
pub const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
/// Per-session ceiling on how many declared files are persisted.
pub const MAX_ARTIFACTS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedArtifact {
    /// The path inside the worker's assigned output directory.
    pub relative_path: String,
    /// The durable absolute path the parent can open directly.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

fn store_root(data_dir: &Path) -> PathBuf {
    data_dir.join("worker-artifacts")
}

/// A session id is a uuid, but never trust one to shape a path.
fn safe_session_directory(session_id: &str) -> String {
    let cleaned = session_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect::<String>();
    if cleaned.is_empty() {
        "unknown-session".into()
    } else {
        cleaned
    }
}

/// A declared output path, as a relative path that cannot traverse. `..`, a
/// root, and a prefix are all refused: the sandbox contract already rejects
/// them, and this is the second, independent check before a durable write.
fn safe_relative(declared: &str) -> Option<PathBuf> {
    let path = Path::new(declared.trim());
    if declared.trim().is_empty() || path.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Copy every declared artifact that exists as a regular file under
/// `output_dir` into `<data_dir>/worker-artifacts/<session>/`.
///
/// Symlinks are refused rather than followed: a link planted in the output tree
/// must not pull an arbitrary host file into the durable store. A missing
/// artifact is simply absent from the result, so the parent is never handed a
/// reference that was never written.
pub fn persist(
    output_dir: &Path,
    declared: &[String],
    session_id: &str,
    data_dir: &Path,
) -> Vec<PersistedArtifact> {
    let destination_root = store_root(data_dir).join(safe_session_directory(session_id));
    let mut persisted = Vec::new();
    for declared_path in declared.iter().take(MAX_ARTIFACTS) {
        let Some(relative) = safe_relative(declared_path) else {
            continue;
        };
        let source = output_dir.join(&relative);
        let Ok(metadata) = std::fs::symlink_metadata(&source) else {
            continue;
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
            continue;
        }
        let Ok(mut file) = std::fs::File::open(&source) else {
            continue;
        };
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        if file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let destination = destination_root.join(&relative);
        let Some(parent) = destination.parent() else {
            continue;
        };
        if std::fs::create_dir_all(parent).is_err() {
            continue;
        }
        if std::fs::write(&destination, &bytes).is_err() {
            continue;
        }
        persisted.push(PersistedArtifact {
            relative_path: relative.to_string_lossy().replace('\\', "/"),
            path: destination.to_string_lossy().into_owned(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    persisted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(path: &str) -> Vec<String> {
        vec![path.to_string()]
    }

    #[test]
    fn declared_artifacts_survive_the_sandbox_and_are_readable_by_path() {
        let sandbox = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("proof.json"),
            r#"{"marker":"ORCHESTRATOR_ARTIFACT_OK"}"#,
        )
        .unwrap();

        let persisted = persist(sandbox.path(), &declared("proof.json"), "worker-1", data.path());
        assert_eq!(persisted.len(), 1);
        let artifact = &persisted[0];
        // The transient tree can disappear; the returned reference still reads.
        std::fs::remove_dir_all(sandbox.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&artifact.path).unwrap(),
            r#"{"marker":"ORCHESTRATOR_ARTIFACT_OK"}"#
        );
        assert_eq!(artifact.bytes, 37);
        assert_eq!(artifact.sha256.len(), 64);
        assert!(std::path::Path::new(&artifact.path).starts_with(data.path().join("worker-artifacts")));
    }

    #[test]
    fn traversal_outside_the_output_directory_is_never_persisted() {
        let workspace = tempfile::tempdir().unwrap();
        let output = workspace.path().join("output");
        let data = workspace.path().join("data");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(workspace.path().join("secret.txt"), "host secret").unwrap();

        let persisted = persist(
            &output,
            &declared("../secret.txt"),
            "worker-1",
            &data,
        );
        assert!(persisted.is_empty());

        let persisted = persist(
            &output,
            &declared("/etc/passwd"),
            "worker-1",
            &data,
        );
        assert!(persisted.is_empty());
    }

    #[test]
    fn a_symlink_planted_in_the_output_tree_is_refused() {
        let workspace = tempfile::tempdir().unwrap();
        let output = workspace.path().join("output");
        let data = workspace.path().join("data");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(workspace.path().join("secret.txt"), "host secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(workspace.path().join("secret.txt"), output.join("report.txt"))
            .unwrap();

        #[cfg(unix)]
        assert!(persist(&output, &declared("report.txt"), "worker-1", &data).is_empty());
    }

    #[test]
    fn an_absent_declared_artifact_is_not_invented() {
        let output = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        assert!(persist(output.path(), &declared("never-written.json"), "w", data.path()).is_empty());
    }

    #[test]
    fn a_session_id_that_looks_like_a_path_cannot_escape_the_store() {
        let output = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("report.md"), "ok").unwrap();
        let persisted = persist(output.path(), &declared("report.md"), "../../escape", data.path());
        assert_eq!(persisted.len(), 1);
        assert!(std::path::Path::new(&persisted[0].path)
            .starts_with(data.path().join("worker-artifacts")));
    }
}
