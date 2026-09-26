//! Run the official installer without terminal prompts or an unbounded wait.
use crate::{adapters, binary, BridgeError};
use std::{
    io::{Read, Seek, SeekFrom},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const INSTALL_TIMEOUT: Duration = Duration::from_secs(240);
const OUTPUT_TAIL_BYTES: u64 = 8 * 1024;
const INSTALL_COMMAND: &str =
    "curl -fsSL --connect-timeout 10 --max-time 30 https://chatgpt.com/codex/install.sh | sh";

fn installer_command() -> Command {
    let mut command = Command::new("/bin/bash");
    binary::hydrate_command_path(&mut command);
    // stdin=null is insufficient: the installer can open /dev/tty directly
    // to offer uninstalling another copy or launching Codex after installation.
    command.env("CODEX_NON_INTERACTIVE", "1");
    command.env_remove("BASH_ENV");
    // A failed download must not look successful because sh got empty input.
    command.args([
        "--noprofile",
        "--norc",
        "-o",
        "pipefail",
        "-c",
        INSTALL_COMMAND,
    ]);
    command
}

pub fn install() -> Result<(), BridgeError> {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err(BridgeError::Invalid(
            "A Codex update is already running".into(),
        ));
    }
    struct ResetRunning;
    impl Drop for ResetRunning {
        fn drop(&mut self) {
            RUNNING.store(false, Ordering::Release);
        }
    }
    let _reset = ResetRunning;
    let started = Instant::now();
    run_installer(&mut installer_command(), || {
        started.elapsed() >= INSTALL_TIMEOUT
    })
}

// A file avoids pipe backpressure and EOF waits on inherited descriptors. Keep
// only a bounded tail in error messages, including diagnostics printed to stdout.
fn run_installer(
    command: &mut Command,
    mut expired: impl FnMut() -> bool,
) -> Result<(), BridgeError> {
    let mut log = tempfile::tempfile()?;
    adapters::configure_process_group(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?)
        .spawn()
        .map_err(|error| {
            BridgeError::Invalid(format!("Could not start the Codex updater: {error}"))
        })?;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(format!("Codex update failed: {status}"))
                }
            }
            Ok(None) if !expired() => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                stop_and_reap(child);
                break Err(
                    "Codex update timed out after 4 minutes. Check your connection and try again."
                        .into(),
                );
            }
            Err(error) => {
                stop_and_reap(child);
                break Err(format!("Could not wait for the Codex updater: {error}"));
            }
        }
    };
    result.map_err(|message| {
        let detail = output_tail(&mut log).unwrap_or_default();
        BridgeError::Invalid(if detail.trim().is_empty() {
            message
        } else {
            format!("{message}\n{}", detail.trim())
        })
    })
}

fn output_tail(log: &mut std::fs::File) -> std::io::Result<String> {
    let length = log.metadata()?.len();
    log.seek(SeekFrom::Start(length.saturating_sub(OUTPUT_TAIL_BYTES)))?;
    let mut tail = Vec::new();
    log.take(OUTPUT_TAIL_BYTES).read_to_end(&mut tail)?;
    Ok(String::from_utf8_lossy(&tail).into_owned())
}

fn stop_and_reap(mut child: Child) {
    // Kill the entire pipeline (including a curl/lock helper started by the
    // installer). Give its cleanup trap a chance to release the install lock
    // before the bounded TERM/KILL escalation.
    let _ = adapters::terminate_process_group(child.id());
    let _ = child.kill();
    // An uninterruptible filesystem must not extend the caller's deadline.
    thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fake_download(script: &str) -> (tempfile::TempDir, Command) {
        let root = tempfile::tempdir().unwrap();
        let curl = root.path().join("curl");
        fs::write(&curl, script).unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = installer_command();
        command.env("PATH", format!("{}:/usr/bin:/bin", root.path().display()));
        (root, command)
    }

    #[test]
    fn installer_receives_noninteractive_mode() {
        let (_root, mut command) = fake_download("#!/bin/sh\ncat <<'INSTALLER'\n[ \"$CODEX_NON_INTERACTIVE\" = 1 ] || { echo 'hidden terminal prompt' >&2; exit 1; }\necho installed\nINSTALLER\n");
        run_installer(&mut command, || false).unwrap();
    }

    #[test]
    fn failed_download_is_not_masked_by_the_shell() {
        let (_root, mut command) =
            fake_download("#!/bin/sh\necho 'download unavailable' >&2\nexit 22\n");
        let error = run_installer(&mut command, || false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("download unavailable"), "{error}");
    }

    #[test]
    fn installer_failures_include_stdout_diagnostics() {
        let (_root, mut command) =
            fake_download("#!/bin/sh\nprintf 'echo install-failed\\nexit 1\\n'\n");
        let error = run_installer(&mut command, || false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("install-failed"), "{error}");
    }

    #[test]
    fn deadline_stops_the_installer_and_its_child() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("child-pid");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "sleep 60 & printf '%s' $! > \"$1\"; wait",
                "installer",
            ])
            .arg(&ready);
        let watchdog = Instant::now();
        // Expire only after the child exists; machine load cannot race setup.
        let error = run_installer(&mut command, || {
            fs::read_to_string(&ready).is_ok_and(|pid| pid.parse::<u32>().is_ok())
                || watchdog.elapsed() > Duration::from_secs(5)
        })
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        let pid = fs::read_to_string(ready).unwrap();
        let stopped = Instant::now();
        loop {
            let state = Command::new("ps")
                .args(["-p", pid.trim(), "-o", "stat="])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&state.stdout);
            if state.trim().is_empty() || state.trim().starts_with('Z') {
                break;
            }
            assert!(
                stopped.elapsed() < Duration::from_secs(5),
                "installer child survived: {state}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
