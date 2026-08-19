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
    /// Task-level network: whether the worker's *tools* may use the network
    /// (drives the provider's own sandbox policy and the briefing).
    network_allowed: bool,
    /// OS-level egress: whether the seatbelt denies all network. Distinct
    /// from `network_allowed` — the provider runtime itself is cloud-backed
    /// and dies on its first model API call without egress.
    runtime_network_denied: bool,
}

impl ReadOnlySandbox {
    pub fn create(
        _session_id: &str,
        workspace: &Path,
        request: &DelegationRequest,
    ) -> Result<Self, BridgeError> {
        Self::create_with_runtime_network(_session_id, workspace, request, runtime_network_denied())
    }

    fn create_with_runtime_network(
        _session_id: &str,
        workspace: &Path,
        request: &DelegationRequest,
        runtime_network_denied: bool,
    ) -> Result<Self, BridgeError> {
        if request.network_access && runtime_network_denied {
            return Err(BridgeError::Invalid("BRIDGE_READ_ONLY_NETWORK=deny forbids read-only worker network egress, but this request asked for network access".into()));
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
            let profile = seatbelt_profile(&workspace, &output, runtime_network_denied)?;
            fs::write(&profile_path, profile)?;
            Ok(Self {
                root_dir: root.clone(),
                profile_path,
                output_dir,
                network_allowed: request.network_access,
                runtime_network_denied,
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

    pub fn runtime_network_denied(&self) -> bool {
        self.runtime_network_denied
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

/// Whether the seatbelt denies ALL network to read-only workers.
///
/// Off by default on purpose: every supported provider runtime is
/// cloud-backed, so `(deny network*)` kills the worker on its first model
/// API call — the CLI boots, then exits before producing anything, and every
/// research delegation fails. Network is therefore on by default at both
/// levels: the runtime may reach its API, and a delegation may request
/// task-level network without any pre-set environment. The single knob is
/// `BRIDGE_READ_ONLY_NETWORK=deny`, which restores total denial for
/// installations running fully local runtimes.
fn runtime_network_denied() -> bool {
    runtime_network_policy_denies(std::env::var("BRIDGE_READ_ONLY_NETWORK").ok().as_deref())
}

fn runtime_network_policy_denies(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("deny") | Some("denied"))
}

fn seatbelt_profile(
    workspace: &Path,
    output: &Path,
    runtime_network_denied: bool,
) -> Result<String, BridgeError> {
    let _workspace = quoted(workspace)?;
    let output = quoted(output)?;
    let network = if runtime_network_denied {
        "(deny network*)"
    } else {
        ""
    };
    Ok(format!(
        r#"(version 1)
(allow default)
; Preserve the provider runtime's non-filesystem IPC and system services while
; denying every write outside this per-worker output directory. Character
; devices are exempted: spawning a child with an ignored stdio stream opens
; /dev/null for writing inside posix_spawn, so a blanket deny makes every
; such spawn fail with EPERM before the tool even runs (the Claude Agent SDK
; launches its CLI exactly that way). Writes to null/zero/ptys are IPC, not
; workspace mutation.
(deny file-write* (require-all
  (require-not (subpath {output}))
  (require-not (literal "/dev/null"))
  (require-not (literal "/dev/zero"))
  (require-not (literal "/dev/ptmx"))
  (require-not (regex #"^/dev/ttys[0-9]+$"))))
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
    fn profile_denies_workspace_writes_but_keeps_provider_egress_by_default() {
        // The write-deny is the isolation boundary. Egress stays open by
        // default because cloud provider runtimes die on their first model
        // API call without it — the failure mode that broke every research
        // worker in the field.
        let profile =
            seatbelt_profile(Path::new("/repo"), Path::new("/tmp/output"), false).unwrap();
        assert!(profile.contains("(allow default)"));
        assert!(profile.contains("(require-not (subpath \"/tmp/output\"))"));
        assert!(profile.contains("(require-not (literal \"/dev/null\"))"));
        assert!(!profile.contains("(deny network*)"));

        let hard_isolated =
            seatbelt_profile(Path::new("/repo"), Path::new("/tmp/output"), true).unwrap();
        assert!(hard_isolated.contains("(deny network*)"));
    }

    #[test]
    fn runtime_network_denial_is_an_explicit_opt_in() {
        assert!(!runtime_network_policy_denies(None));
        assert!(!runtime_network_policy_denies(Some("")));
        assert!(!runtime_network_policy_denies(Some("1")));
        assert!(!runtime_network_policy_denies(Some("allow")));
        assert!(runtime_network_policy_denies(Some("deny")));
        assert!(runtime_network_policy_denies(Some(" denied ")));
    }

    #[test]
    fn a_network_requesting_worker_is_accepted_by_default() {
        // No pre-set environment required: network is on by default at both
        // levels, so a delegation asking for task network just works.
        let workspace = tempfile::tempdir().unwrap();
        let mut networked = request();
        networked.network_access = true;
        let sandbox = ReadOnlySandbox::create_with_runtime_network(
            "test-networked",
            workspace.path(),
            &networked,
            false,
        )
        .unwrap();
        assert!(sandbox.network_allowed());
        assert!(!sandbox.runtime_network_denied());
        sandbox.cleanup();
    }

    #[test]
    fn hard_isolation_refuses_a_network_requesting_worker() {
        let workspace = tempfile::tempdir().unwrap();
        let mut networked = request();
        networked.network_access = true;
        // Even if application policy allowed task network, deny-all egress
        // contradicts it; the contradiction is refused, never half-honored.
        let refused = ReadOnlySandbox::create_with_runtime_network(
            "test",
            workspace.path(),
            &networked,
            true,
        );
        assert!(refused.is_err());
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
    fn dev_null_writes_survive_the_sandbox_so_child_spawns_work() {
        // Spawning a child with an ignored stdio stream opens /dev/null for
        // writing inside posix_spawn; the profile must not turn that into
        // EPERM (it killed every Claude SDK worker in the field).
        if !Path::new("/usr/bin/sandbox-exec").is_file() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = ReadOnlySandbox::create("dev-null", workspace.path(), &request()).unwrap();
        let mut redirect = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
        assert!(redirect
            .args(["-c", "echo probe > /dev/null"])
            .current_dir(sandbox.output_dir())
            .status()
            .unwrap()
            .success());
        sandbox.cleanup();
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
        // Egress denial remains enforceable when explicitly opted into.
        let hard_isolated = ReadOnlySandbox::create_with_runtime_network(
            "test-deny",
            workspace.path(),
            &request(),
            true,
        )
        .unwrap();
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
            let mut network = command(Path::new("/usr/bin/curl"), Some(&hard_isolated)).unwrap();
            assert!(!network
                .args(["-fsS", "--max-time", "2", "https://example.com"])
                .current_dir(hard_isolated.output_dir())
                .status()
                .unwrap()
                .success());
        }
        hard_isolated.cleanup();
        sandbox.cleanup();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires authenticated Codex and Claude runtimes"]
    fn live_codex_and_claude_workers_obey_the_os_boundary() {
        use crate::{
            adapters::{AdapterRuntime, ShutdownReason},
            claude_adapter, codex_adapter,
        };

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
            briefing: None,
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
            briefing: None,
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
