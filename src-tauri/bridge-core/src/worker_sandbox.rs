//! OS-enforced isolation for read-only workers.
//!
//! The provider's own permission flags are useful defense in depth, but this
//! boundary is what prevents shell tools from writing the checked-out tree.
use crate::{delegation::DelegationRequest, BridgeError};
use std::{
    ffi::OsString,
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

/// The OS primitive enforcing the boundary, as recorded in the event ledger.
pub const ISOLATION_MODE: &str = if cfg!(target_os = "linux") { "landlock" } else { "seatbelt" };

/// Constructs the command through the platform primitive. Unsupported systems
/// deliberately return an error instead of silently falling back to audit-only.
pub fn command(program: &Path, sandbox: Option<&ReadOnlySandbox>) -> Result<Command, BridgeError> {
    let Some(sandbox) = sandbox else {
        return Ok(crate::adapters::supervised_command(
            program,
            std::iter::empty::<OsString>(),
        ));
    };
    #[cfg(target_os = "macos")]
    {
        let runner = Path::new("/usr/bin/sandbox-exec");
        if !runner.is_file() {
            return Err(BridgeError::Invalid("Read-only workers require macOS sandbox-exec, but it is unavailable; refusing to start without isolation".into()));
        }
        Ok(crate::adapters::supervised_command(
            runner,
            [
                OsString::from("-f"),
                sandbox.profile_path.as_os_str().to_owned(),
                OsString::from("--"),
                program.as_os_str().to_owned(),
            ],
        ))
    }
    #[cfg(target_os = "linux")]
    {
        let output = sandbox.output_dir.canonicalize()?;
        let ruleset = landlock::ruleset(&output, sandbox.runtime_network_denied)?;
        let mut command = crate::adapters::supervised_command(program, std::iter::empty::<OsString>());
        landlock::restrict_child(&mut command, ruleset);
        Ok(command)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (program, sandbox);
        Err(BridgeError::Invalid("Read-only worker isolation is not available on this platform; refusing to start without isolation".into()))
    }
}

/// Whether a kernel's Landlock ABI can carry the read-only boundary. Below 3,
/// `truncate(2)` is not a handled right, so a worker could still empty any
/// file it can open; below 4 there is no TCP rule, so an explicit network
/// denial cannot be honored. Both refuse rather than half-isolate.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn landlock_supports(abi: i64, deny_network: bool) -> Result<(), BridgeError> {
    if abi < 1 {
        return Err(BridgeError::Invalid("Read-only workers require Linux Landlock, but this kernel does not provide it; refusing to start without isolation".into()));
    }
    if abi < 3 {
        return Err(BridgeError::Invalid(format!("Read-only workers require Landlock ABI 3 or newer (Linux 6.2+), but this kernel provides ABI {abi}; refusing to start without isolation")));
    }
    if deny_network && abi < 4 {
        return Err(BridgeError::Invalid(format!("BRIDGE_READ_ONLY_NETWORK=deny requires Landlock ABI 4 or newer (Linux 6.7+), but this kernel provides ABI {abi}; refusing to start without isolation")));
    }
    Ok(())
}

/// Landlock, Linux's unprivileged sandbox, as the read-only boundary.
///
/// The ruleset is built in the parent, where allocation is safe, and handed to
/// the child as one fd. Between fork and exec the child only calls
/// `prctl(PR_SET_NO_NEW_PRIVS)` and `landlock_restrict_self`, both
/// async-signal-safe, so a multithreaded parent cannot deadlock it. The
/// restriction is inherited across exec by the watchdog shell and everything
/// the provider runtime spawns. Mirrors the seatbelt profile: read and execute
/// anywhere, write only in the worker's output dir and to the character
/// devices a child spawn needs.
#[cfg(target_os = "linux")]
mod landlock {
    use super::{landlock_supports, BridgeError};
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        os::unix::{ffi::OsStrExt, process::CommandExt},
        path::Path,
        process::Command,
    };

    const CREATE_RULESET_VERSION: u32 = 1;
    const RULE_PATH_BENEATH: libc::c_int = 1;

    const EXECUTE: u64 = 1 << 0;
    const WRITE_FILE: u64 = 1 << 1;
    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const REFER: u64 = 1 << 13;
    const TRUNCATE: u64 = 1 << 14;
    const IOCTL_DEV: u64 = 1 << 15;
    const NET_BIND_TCP: u64 = 1 << 0;
    const NET_CONNECT_TCP: u64 = 1 << 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    pub fn abi() -> i64 {
        // SAFETY: the version query takes no attribute and returns an int.
        unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<RulesetAttr>(), 0usize, CREATE_RULESET_VERSION) }
    }

    fn handled_fs(abi: i64) -> u64 {
        let mut rights = (1 << 13) - 1; // ABI 1: EXECUTE..MAKE_SYM
        if abi >= 2 {
            rights |= REFER;
        }
        if abi >= 3 {
            rights |= TRUNCATE;
        }
        if abi >= 5 {
            rights |= IOCTL_DEV;
        }
        rights
    }

    fn os_error(what: &str) -> BridgeError {
        BridgeError::Invalid(format!("Landlock {what} failed: {}; refusing to start without isolation", std::io::Error::last_os_error()))
    }

    fn allow(ruleset: &OwnedFd, path: &Path, access: u64) -> Result<(), BridgeError> {
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            return Err(BridgeError::Invalid("sandbox path contains a NUL byte".into()));
        };
        // SAFETY: a valid C string; O_PATH opens without reading or writing.
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            // A device that does not exist here (no /dev/tty in a container)
            // needs no exemption; anything else is a real failure.
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            return Err(os_error(&format!("open {}", path.display())));
        }
        // SAFETY: `fd` was just returned by open and is owned here.
        let parent = unsafe { OwnedFd::from_raw_fd(fd) };
        let is_dir = path.is_dir();
        // Landlock rejects directory-only rights on a file rule.
        let access = if is_dir { access } else { access & (EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV) };
        let attr = PathBeneathAttr { allowed_access: access, parent_fd: parent.as_raw_fd() };
        // SAFETY: `attr` outlives the call and matches the kernel layout.
        let added = unsafe { libc::syscall(libc::SYS_landlock_add_rule, ruleset.as_raw_fd(), RULE_PATH_BENEATH, &attr as *const PathBeneathAttr, 0u32) };
        if added != 0 {
            return Err(os_error(&format!("rule for {}", path.display())));
        }
        Ok(())
    }

    /// The ruleset fd for a worker that may write only under `output`.
    pub fn ruleset(output: &Path, deny_network: bool) -> Result<OwnedFd, BridgeError> {
        let abi = abi();
        landlock_supports(abi, deny_network)?;
        let fs = handled_fs(abi);
        let attr = RulesetAttr {
            handled_access_fs: fs,
            handled_access_net: if deny_network { NET_BIND_TCP | NET_CONNECT_TCP } else { 0 },
        };
        // ABI 1-3 only know the first field; passing the larger struct to an
        // older kernel is rejected unless the extra bytes are zero, so size
        // the attribute to what the kernel understands.
        let size = if abi >= 4 { std::mem::size_of::<RulesetAttr>() } else { std::mem::size_of::<u64>() };
        // SAFETY: `attr` is valid for `size` bytes.
        let fd = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr as *const RulesetAttr, size, 0u32) };
        if fd < 0 {
            return Err(os_error("ruleset creation"));
        }
        // SAFETY: the syscall returned a new owned fd (O_CLOEXEC by default).
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let read = EXECUTE | READ_FILE | READ_DIR;
        allow(&ruleset, Path::new("/"), read)?;
        // Full rights inside the output dir. A symlink made there cannot widen
        // anything: Landlock checks every access against the resolved path.
        allow(&ruleset, output, fs)?;
        let device = READ_FILE | WRITE_FILE | (fs & IOCTL_DEV);
        for path in ["/dev/null", "/dev/zero", "/dev/ptmx", "/dev/tty", "/dev/full", "/dev/urandom", "/dev/random"] {
            allow(&ruleset, Path::new(path), device)?;
        }
        allow(&ruleset, Path::new("/dev/pts"), device | READ_DIR)?;
        Ok(ruleset)
    }

    /// Apply `ruleset` to the child between fork and exec. If either call
    /// fails the spawn fails, so a worker never runs unrestricted.
    pub fn restrict_child(command: &mut Command, ruleset: OwnedFd) {
        let fd = ruleset.as_raw_fd();
        // SAFETY: the closure only makes two async-signal-safe syscalls. The
        // fd stays open because the closure owns `ruleset` for as long as the
        // command exists; O_CLOEXEC closes it in the child at exec.
        unsafe {
            command.pre_exec(move || {
                let _keep = &ruleset;
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
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

/// Resolve the host's GitHub CLI token so a sandboxed, networked worker's `gh`
/// can authenticate. Inside the read-only sandbox provider-specific writable
/// config and temp roots are redirected, and the macOS keychain is out of reach,
/// so `gh` finds no credentials of its own and every call —
/// even reading a private PR — comes back 401. Passing the token as `GH_TOKEN`
/// is the one thing that lets a review worker read the PR and post its comment.
///
/// Best-effort: `None` when `gh` is missing or logged out, so a machine without
/// GitHub set up is unaffected. This runs from the unsandboxed core process,
/// which still has the keychain — the only place the token is reachable.
pub fn github_cli_token() -> Option<String> {
    let gh = which::which("gh").ok()?;
    let output = std::process::Command::new(gh)
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!token.is_empty()).then_some(token)
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

    #[cfg(unix)]
    #[test]
    fn provider_commands_use_the_parent_death_watchdog_with_and_without_seatbelt() {
        if std::env::var_os(crate::adapters::PARENT_WATCHDOG_DISABLE_ENV).is_some() {
            return;
        }
        let direct = command(Path::new("/bin/sh"), None).unwrap();
        assert_eq!(direct.get_program(), Path::new("/bin/sh"));
        let direct_args = direct
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(direct_args.iter().any(|value| value == "bridge-watchdog"));

        #[cfg(target_os = "macos")]
        {
            let workspace = tempfile::tempdir().unwrap();
            let sandbox = ReadOnlySandbox::create("watchdog", workspace.path(), &request()).unwrap();
            let wrapped = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
            let args = wrapped
                .get_args()
                .map(|value| value.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert!(args.iter().any(|value| value == "/usr/bin/sandbox-exec"));
            assert!(args.iter().any(|value| value == "/bin/sh"));
            sandbox.cleanup();
        }
    }

    #[test]
    fn landlock_refuses_kernels_that_cannot_hold_the_boundary() {
        assert!(landlock_supports(-1, false).unwrap_err().to_string().contains("does not provide it"));
        assert!(landlock_supports(0, false).is_err());
        assert!(landlock_supports(2, false).unwrap_err().to_string().contains("ABI 3"));
        assert!(landlock_supports(3, false).is_ok());
        assert!(landlock_supports(3, true).unwrap_err().to_string().contains("ABI 4"));
        assert!(landlock_supports(4, true).is_ok());
        assert!(landlock_supports(6, true).is_ok());
    }

    #[cfg(target_os = "linux")]
    fn landlock_available() -> bool {
        let abi = landlock::abi();
        if abi < 3 {
            eprintln!("skipping: Landlock ABI {abi} on this kernel");
        }
        abi >= 3
    }

    #[cfg(target_os = "linux")]
    fn sandboxed_sh(sandbox: &ReadOnlySandbox, script: &str) -> std::process::ExitStatus {
        command(Path::new("/bin/sh"), Some(sandbox))
            .unwrap()
            .args(["-c", script])
            .current_dir(sandbox.output_dir())
            .env("BRIDGE_WORKER_OUTPUT_DIR", sandbox.output_dir())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_prevents_workspace_writes_but_allows_assigned_output() {
        if !landlock_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("tracked.txt"), "original").unwrap();
        let sandbox = ReadOnlySandbox::create("landlock", workspace.path(), &request()).unwrap();

        let target = workspace.path().join("blocked.txt");
        assert!(!sandboxed_sh(&sandbox, &format!("touch '{}'", target.display())).success());
        assert!(!target.exists());
        // Overwrite and truncate of an existing file are both denied (ABI 3+).
        let tracked = workspace.path().join("tracked.txt");
        assert!(!sandboxed_sh(&sandbox, &format!("echo changed > '{}'", tracked.display())).success());
        assert!(!sandboxed_sh(&sandbox, &format!("truncate -s 0 '{}'", tracked.display())).success());
        assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "original");

        // Reads and execution still work, and the output dir is writable.
        assert!(sandboxed_sh(&sandbox, &format!("cat '{}' > \"$BRIDGE_WORKER_OUTPUT_DIR/copy.txt\"", tracked.display())).success());
        assert_eq!(std::fs::read_to_string(sandbox.output_dir().join("copy.txt")).unwrap(), "original");
        assert!(sandboxed_sh(&sandbox, "mkdir -p \"$BRIDGE_WORKER_OUTPUT_DIR/a/b\" && touch \"$BRIDGE_WORKER_OUTPUT_DIR/a/b/c\"").success());
        sandbox.cleanup();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_write_failures_are_eacces() {
        if !landlock_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = ReadOnlySandbox::create("landlock-errno", workspace.path(), &request()).unwrap();
        let output = command(Path::new("/bin/sh"), Some(&sandbox))
            .unwrap()
            .args(["-c", &format!("touch '{}'", workspace.path().join("x").display())])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Permission denied"), "expected EACCES, got: {stderr}");
        sandbox.cleanup();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_keeps_dev_null_writable_so_child_spawns_work() {
        if !landlock_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let sandbox = ReadOnlySandbox::create("landlock-null", workspace.path(), &request()).unwrap();
        assert!(sandboxed_sh(&sandbox, "echo probe > /dev/null && head -c 4 /dev/zero > /dev/null").success());
        sandbox.cleanup();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn landlock_denies_tcp_only_when_runtime_network_is_denied() {
        if !landlock_available() || landlock::abi() < 4 {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || for stream in listener.incoming() { drop(stream); });
        // bash's /dev/tcp is a plain connect(2), no extra tooling needed.
        let Ok(bash) = which::which("bash") else { return };
        let connect = |sandbox: &ReadOnlySandbox| {
            command(&bash, Some(sandbox))
                .unwrap()
                .args(["-c", &format!("exec 3<>/dev/tcp/127.0.0.1/{port}")])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        let workspace = tempfile::tempdir().unwrap();
        let open = ReadOnlySandbox::create_with_runtime_network("net-open", workspace.path(), &request(), false).unwrap();
        assert!(connect(&open), "default sandbox keeps provider egress");
        let denied = ReadOnlySandbox::create_with_runtime_network("net-deny", workspace.path(), &request(), true).unwrap();
        assert!(!connect(&denied), "BRIDGE_READ_ONLY_NETWORK=deny blocks TCP");
        open.cleanup();
        denied.cleanup();
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
    fn projected_capabilities_are_readable_but_the_host_sources_remain_immutable() {
        if !Path::new("/usr/bin/sandbox-exec").is_file() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let source_skill = home.path().join(".claude/skills/review/SKILL.md");
        fs::create_dir_all(source_skill.parent().unwrap()).unwrap();
        fs::write(&source_skill, "projected capability").unwrap();
        let sandbox = ReadOnlySandbox::create("capabilities", workspace.path(), &request()).unwrap();
        crate::capability_projection::project_read_only_capabilities_with_environment(
            crate::capability_projection::CapabilityHarness::Claude,
            home.path(),
            sandbox.output_dir(),
            &crate::capability_projection::CapabilityEnvironment::default(),
        )
        .unwrap();

        let mut read = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
        assert!(read
            .args(["-c", "test \"$(cat \"$CLAUDE_CONFIG_DIR/skills/review/SKILL.md\")\" = 'projected capability'"])
            .env("CLAUDE_CONFIG_DIR", sandbox.output_dir().join(".claude"))
            .current_dir(sandbox.output_dir())
            .status()
            .unwrap()
            .success());

        let mut write = command(Path::new("/bin/sh"), Some(&sandbox)).unwrap();
        assert!(!write
            .args(["-c", "printf changed >> \"$CLAUDE_CONFIG_DIR/skills/review/SKILL.md\""])
            .env("CLAUDE_CONFIG_DIR", sandbox.output_dir().join(".claude"))
            .current_dir(sandbox.output_dir())
            .status()
            .unwrap()
            .success());
        assert_eq!(fs::read_to_string(source_skill).unwrap(), "projected capability");
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
            interactive: false,
            on_progress: None,
        })
        .unwrap();
        codex.runtime
            .start_turn(prompt, crate::adapters::TurnContext::default())
            .unwrap();
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
            interactive: false,
            on_progress: None,
        })
        .unwrap();
        claude.runtime
            .start_turn(prompt, crate::adapters::TurnContext::default())
            .unwrap();
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
