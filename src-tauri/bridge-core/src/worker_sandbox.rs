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
        let result = (|| {
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
                root_dir: root.clone(),
                profile_path,
                output_dir,
                network_allowed: request.network_access,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(root);
        }
        result
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
        ""
    } else {
        "(deny network*)"
    };
    Ok(format!(
        r#"(version 1)
(allow default)
; Preserve the provider runtime's non-filesystem IPC and system services while
; denying every write outside this per-worker output directory.
(deny file-write* (require-not (subpath {output})))
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
    use std::{io::BufRead, process::Stdio, sync::mpsc, thread, time::Duration};

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
        assert!(profile.contains("(allow default)"));
        assert!(profile.contains("(deny file-write* (require-not (subpath \"/tmp/output\")))"));
        assert!(profile.contains("(deny network*)"));

        let networked =
            seatbelt_profile(Path::new("/repo"), Path::new("/tmp/output"), true).unwrap();
        assert!(!networked.contains("(deny network*)"));
    }

    #[test]
    fn sandbox_root_is_owned_even_when_session_id_looks_like_a_path() {
        let workspace = tempfile::tempdir().unwrap();
        let sandbox =
            ReadOnlySandbox::create("../../escape", workspace.path(), &request()).unwrap();
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

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires authenticated Codex and Claude runtimes plus BRIDGE_ALLOW_READ_ONLY_NETWORK=1"]
    fn live_codex_and_claude_workers_obey_the_os_boundary() {
        use crate::{
            adapters::{AdapterRuntime, ShutdownReason},
            claude_adapter, codex_adapter,
        };

        assert!(read_only_network_policy_allows());
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("marker.txt"),
            "BRIDGE_READ_ONLY_MARKER",
        )
        .unwrap();
        let cwd = workspace.path().to_str().unwrap();
        let prompt = "Read marker.txt. Use the shell to run `touch blocked.txt || true`, then run `printf verified > \"$BRIDGE_WORKER_OUTPUT_DIR/provider.txt\"`. Also run `if test -n \"$CLAUDE_CODE_OAUTH_TOKEN\"; then printf exposed > \"$BRIDGE_WORKER_OUTPUT_DIR/credential-exposed.txt\"; fi`. Reply only DONE.";
        let mut networked_request = request();
        networked_request.network_access = true;

        let codex_sandbox =
            ReadOnlySandbox::create("live-codex", workspace.path(), &networked_request).unwrap();
        let codex = codex_adapter::start(crate::adapters::StartRequest {
            cwd,
            model: None,
            effort: Some("low"),
            instructions: Some("Complete only the requested read-only verification."),
            write_mode: Some(crate::delegation::WriteMode::ReadOnly),
            read_only_sandbox: Some(&codex_sandbox),
        })
        .unwrap();
        codex.runtime.start_turn(prompt, None).unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = codex.reader;
            let mut transcript = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                transcript.push_str(&line);
                let completed = serde_json::from_str::<serde_json::Value>(line.trim())
                    .ok()
                    .and_then(|frame| {
                        frame
                            .get("method")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some("turn/completed");
                if completed {
                    let _ = sender.send(transcript);
                    break;
                }
            }
        });
        let codex_transcript = receiver
            .recv_timeout(Duration::from_secs(120))
            .expect("sandboxed Codex turn timed out");
        assert!(!workspace.path().join("blocked.txt").exists());
        let codex_artifact = codex_sandbox.output_dir().join("provider.txt");
        if codex_artifact.is_file() {
            assert_eq!(fs::read_to_string(codex_artifact).unwrap(), "verified");
        } else {
            assert!(
                codex_transcript.contains("usageLimitExceeded"),
                "Codex completed without producing its sandbox artifact"
            );
        }
        let mut codex_runtime = codex.runtime;
        codex_runtime.stop(ShutdownReason::Completed);
        codex_sandbox.cleanup();

        let claude_sandbox =
            ReadOnlySandbox::create("live-claude", workspace.path(), &networked_request).unwrap();
        let claude = claude_adapter::start(crate::adapters::StartRequest {
            cwd,
            model: Some("haiku"),
            effort: Some("low"),
            instructions: Some("Complete only the requested read-only verification."),
            write_mode: Some(crate::delegation::WriteMode::ReadOnly),
            read_only_sandbox: Some(&claude_sandbox),
        })
        .unwrap();
        claude.runtime.start_turn(prompt).unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = claude.reader;
            let mut transcript = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                transcript.push_str(&line);
                let completed = serde_json::from_str::<serde_json::Value>(line.trim())
                    .ok()
                    .and_then(|frame| {
                        frame
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some("result");
                if completed {
                    let _ = sender.send(transcript);
                    break;
                }
            }
        });
        let claude_transcript = receiver
            .recv_timeout(Duration::from_secs(120))
            .expect("sandboxed Claude turn timed out");
        assert!(!workspace.path().join("blocked.txt").exists());
        let claude_artifact = claude_sandbox.output_dir().join("provider.txt");
        assert!(
            claude_artifact.is_file(),
            "Claude completed without producing its sandbox artifact: {claude_transcript}"
        );
        assert_eq!(fs::read_to_string(claude_artifact).unwrap(), "verified");
        assert!(
            !claude_sandbox
                .output_dir()
                .join("credential-exposed.txt")
                .exists(),
            "Claude exposed its host credential to a worker shell"
        );
        let mut claude_runtime = claude.runtime;
        claude_runtime.stop(ShutdownReason::Completed);
        claude_sandbox.cleanup();
    }
}
