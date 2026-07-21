//! OS-enforced isolation for read-only workers.
//!
//! The provider's own permission flags are useful defense in depth, but this
//! boundary is what prevents shell tools from writing the checked-out tree.
use crate::{delegation::DelegationRequest, BridgeError};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlySandbox {
    root_dir: PathBuf,
    profile_path: PathBuf,
    output_dir: PathBuf,
    network_allowed: bool,
}

impl ReadOnlySandbox {
    pub fn create(
        _session_id: &str,
        workspace: &Path,
        request: &DelegationRequest,
    ) -> Result<Self, BridgeError> {
        if request.network_access && !read_only_network_policy_allows() {
            return Err(BridgeError::Invalid("Read-only worker requested network access, but Bridge policy does not authorize it".into()));
        }
        let root = std::env::temp_dir()
            .join("bridge-read-only-workers")
            .join(Uuid::new_v4().to_string());
        let output_dir = root.join("output");
        fs::create_dir_all(&output_dir)?;
        let profile_path = root.join("seatbelt.sb");
        let workspace = workspace.canonicalize().map_err(|error| {
            BridgeError::Invalid(format!(
                "Cannot isolate missing workspace {}: {error}",
                workspace.display()
            ))
        })?;
        let output = output_dir.canonicalize()?;
        let profile = seatbelt_profile(&workspace, &output, request.network_access)?;
        fs::write(&profile_path, profile)?;
        Ok(Self {
            root_dir: root,
            profile_path,
            output_dir,
            network_allowed: request.network_access,
        })
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub fn network_allowed(&self) -> bool {
        self.network_allowed
    }

    pub fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.root_dir);
    }
}

/// Constructs the command through the platform primitive. Unsupported systems
/// deliberately return an error instead of silently falling back to audit-only.
pub fn command(program: &Path, sandbox: Option<&ReadOnlySandbox>) -> Result<Command, BridgeError> {
    let Some(sandbox) = sandbox else {
        return Ok(Command::new(program));
    };
    #[cfg(target_os = "macos")]
    {
        let runner = Path::new("/usr/bin/sandbox-exec");
        if !runner.is_file() {
            return Err(BridgeError::Invalid("Read-only workers require macOS sandbox-exec, but it is unavailable; refusing to start without isolation".into()));
        }
        let mut command = Command::new(runner);
        command
            .arg("-f")
            .arg(&sandbox.profile_path)
            .arg("--")
            .arg(program);
        Ok(command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (program, sandbox);
        Err(BridgeError::Invalid("Read-only worker isolation is not available on this platform; refusing to start without isolation".into()))
    }
}

fn read_only_network_policy_allows() -> bool {
    // Deliberately opt-in at the application policy boundary, never by a model
    // request alone. An administrator may set this before launching Bridge.
    matches!(
        std::env::var("BRIDGE_ALLOW_READ_ONLY_NETWORK").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn seatbelt_profile(
    workspace: &Path,
    output: &Path,
    network_allowed: bool,
) -> Result<String, BridgeError> {
    let _workspace = quoted(workspace)?;
    let output = quoted(output)?;
    let network = if network_allowed {
        "(allow network*)"
    } else {
        "(deny network*)"
    };
    Ok(format!(
        r#"(version 1)
(deny default)
(allow process*)
(allow file-read*)
; Default deny makes the repository immutable. Only this per-worker output
; directory is writable, which also becomes HOME/TMPDIR for provider tools.
(allow file-write* (subpath {output}))
{network}
"#
    ))
}

fn quoted(path: &Path) -> Result<String, BridgeError> {
    let value = path
        .to_str()
        .ok_or_else(|| BridgeError::Invalid("workspace path is not valid UTF-8".into()))?;
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    fn request() -> DelegationRequest {
        DelegationRequest {
            schema_version: 1,
            role: crate::delegation::WorkerRole::Verification,
            objective: "inspect".into(),
            acceptance_criteria: vec!["report".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec![],
            owned_paths: vec![],
            write_mode: crate::delegation::WriteMode::ReadOnly,
            capability_tier: crate::model::CapabilityTier::Fast,
            effort: crate::delegation::Effort::Low,
            network_access: false,
            writable_output_paths: vec!["report.txt".into()],
            verification: vec![],
            output_contract: crate::delegation::OutputContract::VerificationResult,
            harness: None,
            model: None,
        }
    }

    #[test]
    fn profile_denies_workspace_writes_and_network_by_default() {
        let profile =
            seatbelt_profile(Path::new("/repo"), Path::new("/tmp/output"), false).unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-write* (subpath \"/tmp/output\"))"));
        assert!(profile.contains("(deny network*)"));

        let networked =
            seatbelt_profile(Path::new("/repo"), Path::new("/tmp/output"), true).unwrap();
        assert!(networked.contains("(allow network*)"));
        assert!(!networked.contains("(deny network*)"));
    }

    #[test]
    fn sandbox_root_is_owned_even_when_session_id_looks_like_a_path() {
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = ReadOnlySandbox::create("../../escape", workspace.path(), &request()).unwrap();
        let expected_parent = std::env::temp_dir().join("bridge-read-only-workers");
        assert_eq!(sandbox.root_dir.parent(), Some(expected_parent.as_path()));
        assert!(sandbox.profile_path.is_file());
        let root = sandbox.root_dir.clone();
        sandbox.cleanup();
        sandbox.cleanup();
        assert!(!root.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_prevents_workspace_writes_but_allows_assigned_output() {
        if !Path::new("/usr/bin/sandbox-exec").is_file() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = ReadOnlySandbox::create("test", workspace.path(), &request()).unwrap();
        let mut output = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
        let target = workspace.path().join("blocked.txt");
        let status = output
            .arg("-c")
            .arg(format!("touch '{}'", target.display()))
            .current_dir(sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success());
        assert!(!workspace.path().join("blocked.txt").exists());
        // The shell stops at the denied write; prove output separately.
        let mut output = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
        assert!(output
            .arg("-c")
            .arg("touch \"$BRIDGE_WORKER_OUTPUT_DIR/allowed.txt\"")
            .current_dir(sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir())
            .status()
            .unwrap()
            .success());
        assert!(sandbox.output_dir().join("allowed.txt").exists());
        if Path::new("/usr/bin/curl").is_file() {
            let mut network = command(Path::new("/usr/bin/curl"), Some(&sandbox)).unwrap();
            assert!(!network
                .args(["-fsS", "--max-time", "2", "https://example.com"])
                .current_dir(sandbox.output_dir())
                .status()
                .unwrap()
                .success());
        }
        sandbox.cleanup();
    }
}
