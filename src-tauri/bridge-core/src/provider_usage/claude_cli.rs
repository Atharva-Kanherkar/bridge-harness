use super::{window, AccountUsage};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::Path,
    sync::{mpsc, Condvar, LazyLock, Mutex},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_LIMIT: usize = 256 * 1024;
const SETTLE: Duration = Duration::from_secs(2);
static ACTIVE_PROBES: LazyLock<ProbeRegistry> = LazyLock::new(ProbeRegistry::default);

#[derive(Default)]
struct ProbeState {
    shutting_down: bool,
    launches: usize,
    active: HashSet<u32>,
}

#[derive(Default)]
struct ProbeRegistry {
    state: Mutex<ProbeState>,
    changed: Condvar,
}

struct LaunchGuard<'a>(&'a ProbeRegistry);
struct ActiveGuard<'a> {
    registry: &'a ProbeRegistry,
    pid: u32,
}

impl ProbeRegistry {
    fn begin_launch(&self) -> Result<LaunchGuard<'_>, String> {
        let mut state = self.state.lock().unwrap();
        if state.shutting_down {
            return Err("Bridge is shutting down; Claude CLI refresh was cancelled.".into());
        }
        state.launches += 1;
        Ok(LaunchGuard(self))
    }
    fn register(&self, pid: u32) -> ActiveGuard<'_> {
        self.state.lock().unwrap().active.insert(pid);
        ActiveGuard {
            registry: self,
            pid,
        }
    }
    fn shutdown(&self) {
        let active = {
            let mut state = self.state.lock().unwrap();
            state.shutting_down = true;
            while state.launches > 0 {
                state = self.changed.wait(state).unwrap();
            }
            state.active.iter().copied().collect::<Vec<_>>()
        };
        for pid in active {
            terminate_pid(pid);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while !state.active.is_empty() && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (next, _) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
        }
    }
}

impl Drop for LaunchGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.launches -= 1;
        self.0.changed.notify_all();
    }
}
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.registry.state.lock().unwrap().active.remove(&self.pid);
        self.registry.changed.notify_all();
    }
}

pub(super) fn shutdown() {
    ACTIVE_PROBES.shutdown();
}

pub(super) fn read(core: &crate::BridgeCore) -> Result<AccountUsage, String> {
    let binary = crate::binary::resolve("claude")
        .ok_or_else(|| "Claude Code is not installed or is not on PATH.".to_string())?;
    let base = core.database_path.parent().ok_or_else(|| {
        "Bridge cannot prepare its private Claude CLI probe directory.".to_string()
    })?;
    let working_directory = base.join("claude-usage-probe");
    std::fs::create_dir_all(&working_directory)
        .map_err(|_| "Bridge cannot prepare its private Claude CLI probe directory.".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&working_directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Bridge cannot protect its Claude CLI probe directory.".to_string())?;
    }
    capture_with_registry(
        &binary,
        &working_directory,
        TIMEOUT,
        OUTPUT_LIMIT,
        &ACTIVE_PROBES,
        None,
    )
}

#[cfg(test)]
fn capture(
    binary: &Path,
    working_directory: &Path,
    timeout: Duration,
    output_limit: usize,
) -> Result<AccountUsage, String> {
    capture_with_registry(
        binary,
        working_directory,
        timeout,
        output_limit,
        &ACTIVE_PROBES,
        None,
    )
}

fn capture_with_registry(
    binary: &Path,
    working_directory: &Path,
    timeout: Duration,
    output_limit: usize,
    registry: &ProbeRegistry,
    spawned: Option<&mpsc::Sender<u32>>,
) -> Result<AccountUsage, String> {
    let launch = registry.begin_launch()?;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 140,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Could not open a Claude CLI terminal: {error}"))?;
    let mut command = CommandBuilder::new(binary);
    command.args([
        "--safe-mode",
        "--tools",
        "",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--session-id",
        &uuid::Uuid::new_v4().to_string(),
        "--ax-screen-reader",
    ]);
    command.env("TERM", "xterm-256color");
    command.env("DISABLE_AUTOUPDATER", "1");
    command.cwd(working_directory);
    command.env("PWD", working_directory);
    if let Some(path) = hydrated_path() {
        command.env("PATH", path);
    }
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Could not read Claude CLI output: {error}"))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Could not send /usage to Claude Code: {error}"))?;
    // Acquire every fallible PTY handle before spawning. After this point the
    // single cleanup path below owns termination and reaping.
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Could not start Claude Code: {error}"))?;
    drop(pair.slave);
    let process_id = child.process_id();
    let active = process_id.map(|pid| registry.register(pid));
    drop(launch);
    if let (Some(spawned), Some(process_id)) = (spawned, process_id) {
        let _ = spawned.send(process_id);
    }
    let (send, receive) = mpsc::sync_channel(8);
    let reader_thread = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if send.send(chunk[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let started = Instant::now();
    let mut output = Vec::new();
    let mut command_sent = false;
    let mut candidate = None;
    let mut candidate_since = None;
    let mut last_clear_at = None;
    let result = loop {
        if started.elapsed() >= timeout {
            break Err(
                "Claude CLI /usage timed out. Open Claude Code, sign in, and try Refresh again."
                    .into(),
            );
        }
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                if output.len().saturating_add(chunk.len()) > output_limit {
                    break Err("Claude CLI /usage produced too much output.".into());
                }
                output.extend_from_slice(&chunk);
                let text = String::from_utf8_lossy(&output);
                if let Some(error) = blocking_prompt(&text) {
                    break Err(error);
                }
                if !command_sent {
                    if ready_for_command(&text) {
                        if let Err(error) =
                            writer.write_all(b"/usage\r").and_then(|_| writer.flush())
                        {
                            break Err(format!("Could not send /usage to Claude Code: {error}"));
                        }
                        command_sent = true;
                    }
                    continue;
                }
                let clear_at = output.windows(4).rposition(|window| window == b"\x1b[2J");
                if clear_at != last_clear_at {
                    candidate = None;
                    candidate_since = None;
                    last_clear_at = clear_at;
                }
                match parse(&text) {
                    Ok(usage) => {
                        if candidate.is_none() {
                            candidate_since = Some(Instant::now());
                        }
                        candidate = Some(usage);
                    }
                    Err(ParseError::Loading) => {}
                    Err(ParseError::Malformed) => {}
                    Err(ParseError::Failed(error)) => break Err(error),
                }
                if candidate.is_some() && candidate_since.is_some_and(|at| at.elapsed() >= SETTLE) {
                    break Ok(candidate.expect("candidate age is set with a candidate"));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break candidate
                    .or_else(|| parse(&String::from_utf8_lossy(&output)).ok())
                    .ok_or_else(|| {
                        "Claude CLI closed before /usage returned quota percentages.".into()
                    });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if candidate.is_some() && candidate_since.is_some_and(|at| at.elapsed() >= SETTLE) {
                    break Ok(candidate.expect("candidate time is set with a candidate"));
                }
            }
        }
    };
    stop_child(process_id, &mut *child);
    drop(writer);
    drop(pair.master);
    drop(receive);
    let _ = reader_thread.join();
    drop(active);
    result
}

fn terminate_pid(process_id: u32) {
    if !crate::adapters::terminate_process_group(process_id) {
        #[cfg(unix)]
        unsafe {
            libc::kill(process_id as i32, libc::SIGKILL);
        }
    }
}

fn stop_child(process_id: Option<u32>, child: &mut dyn portable_pty::Child) {
    if let Some(process_id) = process_id {
        terminate_pid(process_id);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn hydrated_path() -> Option<std::ffi::OsString> {
    let mut command = std::process::Command::new("true");
    crate::binary::hydrate_command_path(&mut command);
    command.get_envs().find_map(|(key, value)| {
        (key == "PATH")
            .then(|| value.map(std::ffi::OsStr::to_os_string))
            .flatten()
    })
}

fn strip_ansi(text: &str) -> String {
    regex::Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))")
        .expect("valid ANSI regex")
        .replace_all(text, "")
        .replace('\r', "\n")
}

fn ready_for_command(text: &str) -> bool {
    strip_ansi(text).lines().any(|line| {
        let line = line.trim();
        line == "❯" || line == ">" || line == "$"
    })
}

fn blocking_prompt(text: &str) -> Option<String> {
    let normalized = strip_ansi(text).to_lowercase();
    let prompts = [
        "do you trust the files in this folder",
        "yes, i trust this folder",
        "press enter to continue",
        "sign in to claude",
        "login required",
        "permission required",
        "allow this action",
        "choose a theme",
        "select a theme",
        "update available",
        "would you like to update",
        "quick safety check",
        "ready to code here",
    ];
    prompts.iter().any(|needle| normalized.contains(needle)).then(|| {
        "Claude Code needs authentication, trust, or permission. Complete it in Claude Code, then try Refresh again; Bridge did not answer the prompt.".into()
    })
}

#[derive(Debug)]
enum ParseError {
    Loading,
    Malformed,
    Failed(String),
}

fn parse(text: &str) -> Result<AccountUsage, ParseError> {
    // A TUI capture retains erased content. Once Claude clears the screen,
    // older percentages are no longer visible and must not be combined with a
    // partially rendered new panel.
    let frame = text
        .rfind("\x1b[2J")
        .map(|index| &text[index + "\x1b[2J".len()..])
        .unwrap_or(text);
    let clean = strip_ansi(frame);
    let lower = clean.to_lowercase();
    if lower.contains("failed to load usage data") {
        return Err(ParseError::Failed(
            "Claude CLI could not load usage data. Open Claude Code, run /usage, and try Refresh again."
                .into(),
        ));
    }
    let session = percent_in_latest_section(
        &clean,
        &["current session", "five hour", "5-hour"],
        &["current week", "weekly"],
    );
    let weekly = weekly_percent(&clean);
    if session.is_none() && weekly.is_none() {
        return if lower.contains("loading") || lower.contains("/usage") {
            Err(ParseError::Loading)
        } else {
            Err(ParseError::Malformed)
        };
    }
    let mut usage = AccountUsage {
        observed_at: chrono::Utc::now().timestamp(),
        source: Some("Claude CLI".into()),
        ..Default::default()
    };
    if let Some(percent) = session {
        usage
            .windows
            .push(window("session", "5-hour", Some(percent), None, Some(300)));
    }
    if let Some(percent) = weekly {
        usage
            .windows
            .push(window("weekly", "Weekly", Some(percent), None, Some(10080)));
    }
    Ok(usage)
}

fn percent_in_latest_section(text: &str, labels: &[&str], other_labels: &[&str]) -> Option<f64> {
    let lower = text.to_ascii_lowercase();
    labels
        .iter()
        .filter_map(|label| lower.rmatch_indices(label).next().map(|(index, _)| index))
        .max()
        .and_then(|index| {
            let after_label = &lower[index..];
            let boundary = other_labels
                .iter()
                .filter_map(|label| after_label[1..].find(label).map(|next| next + 1))
                .min()
                .unwrap_or(after_label.len());
            let boundary = safe_prefix_len(after_label, boundary.min(800));
            let section = &after_label[..boundary];
            regex::Regex::new(r"([0-9]{1,3}(?:\.[0-9]+)?)\s*%\s*used")
                .expect("valid percent regex")
                .captures(section)?
                .get(1)?
                .as_str()
                .parse::<f64>()
                .ok()
        })
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
}

fn weekly_percent(text: &str) -> Option<f64> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("current week (all models)") {
        percent_in_latest_section(
            text,
            &["current week (all models)"],
            &["current week", "current session", "five hour", "5-hour"],
        )
    } else {
        percent_in_latest_section(
            text,
            &["weekly"],
            &["current week", "current session", "five hour", "5-hour"],
        )
    }
}

fn safe_prefix_len(text: &str, maximum: usize) -> usize {
    if maximum >= text.len() {
        return text.len();
    }
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum)
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_redrawn_percentages_without_resets_or_identity() {
        let usage = parse("\x1b[2JCurrent session 12% used\rCurrent week (all models) 34.5% used\x1b[2JCurrent session 18% used\rCurrent week (all models) 41% used").unwrap();
        assert_eq!(usage.windows[0].used_percent.value, Some(18.0));
        assert_eq!(usage.windows[1].used_percent.value, Some(41.0));
        assert!(usage
            .windows
            .iter()
            .all(|window| window.resets_at.is_none()));
        assert_eq!(usage.account, None);
        assert_eq!(usage.plan, None);
        assert_eq!(usage.source.as_deref(), Some("Claude CLI"));
    }

    #[test]
    fn loading_and_malformed_output_never_fabricate_quota() {
        assert!(matches!(
            parse("/usage Loading usage data…"),
            Err(ParseError::Loading)
        ));
        assert!(matches!(
            parse("Current session unknown"),
            Err(ParseError::Malformed)
        ));
        assert!(blocking_prompt("Do you trust the files in this folder?").is_some());
        assert!(matches!(
            parse("Failed to load usage data"),
            Err(ParseError::Failed(_))
        ));
        assert!(matches!(
            parse("Current session 81% remaining"),
            Err(ParseError::Malformed)
        ));
        assert!(ready_for_command("Claude Code\n$\n"));
        assert!(!ready_for_command("$you: /usage"));
        assert!(!ready_for_command("$ Try asking about this repository"));
    }

    #[test]
    fn unicode_and_section_boundaries_do_not_borrow_another_quota() {
        let usage =
            parse("✨ Current session loading…\nCurrent week (all models) 72% used").unwrap();
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].label, "Weekly");
        assert_eq!(usage.windows[0].used_percent.value, Some(72.0));
        assert!(matches!(
            parse("✨ Current sess"),
            Err(ParseError::Malformed)
        ));
        let partial_redraw = parse(
            "Current session 10% used\nCurrent week (all models) 20% used\x1b[2JCurrent session 30% used",
        )
        .unwrap();
        assert_eq!(partial_redraw.windows.len(), 1);
        assert_eq!(partial_redraw.windows[0].label, "5-hour");
        let scoped_after_total =
            parse("Current week (all models) 42% used\nCurrent week (Sonnet) 7% used").unwrap();
        assert_eq!(scoped_after_total.windows.len(), 1);
        assert_eq!(scoped_after_total.windows[0].used_percent.value, Some(42.0));
        let long_unicode = format!("Current session {}✨ 9% used", "x".repeat(790));
        let _ = parse(&long_unicode);
    }

    #[cfg(unix)]
    #[test]
    fn fake_cli_is_reaped_after_a_successful_capture() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("claude-fake");
        let pid_file = directory.path().join("pid");
        std::fs::write(&binary, format!(
            "#!/bin/sh\nprintf '%s' $$ > '{}'\nprintf '>\\n'\nIFS= read -r command\nprintf '\\033[2JCurrent session 27%% used\\rCurrent week (all models) 63%% used\\n'\nwhile :; do :; done\n",
            pid_file.display()
        )).unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let usage = capture(&binary, directory.path(), Duration::from_secs(4), 16 * 1024).unwrap();
        assert_eq!(usage.windows[0].used_percent.value, Some(27.0));
        assert_eq!(usage.windows[1].used_percent.value, Some(63.0));
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "fake CLI child still exists"
        );
    }

    #[cfg(unix)]
    #[test]
    fn startup_prompt_aborts_without_writing_a_command() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("claude-prompt");
        let command_file = directory.path().join("command");
        let pid_file = directory.path().join("pid");
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s' $$ > '{}'\nprintf 'Do you trust the files in this folder?\\n'\nIFS= read -r command\nprintf '%s' \"$command\" > '{}'\n",
                pid_file.display(), command_file.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let error =
            capture(&binary, directory.path(), Duration::from_secs(2), 16 * 1024).unwrap_err();
        assert!(error.contains("did not answer"));
        assert!(!command_file.exists());
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_and_output_cap_return_promptly() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let timeout_binary = directory.path().join("claude-timeout");
        std::fs::write(
            &timeout_binary,
            "#!/bin/sh\nprintf '>\\n'\nIFS= read -r command\nwhile :; do :; done\n",
        )
        .unwrap();
        let cap_binary = directory.path().join("claude-cap");
        std::fs::write(
            &cap_binary,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\nwhile :; do :; done\n",
                "x".repeat(2048)
            ),
        )
        .unwrap();
        for binary in [&timeout_binary, &cap_binary] {
            let mut permissions = std::fs::metadata(binary).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(binary, permissions).unwrap();
        }
        let started = Instant::now();
        let (pid_send, pid_receive) = mpsc::channel();
        let registry = ProbeRegistry::default();
        assert!(capture_with_registry(
            &timeout_binary,
            directory.path(),
            Duration::from_millis(300),
            16 * 1024,
            &registry,
            Some(&pid_send),
        )
        .unwrap_err()
        .contains("timed out"));
        let cap_error = capture_with_registry(
            &cap_binary,
            directory.path(),
            Duration::from_secs(5),
            1024,
            &registry,
            Some(&pid_send),
        )
        .unwrap_err();
        assert!(cap_error.contains("too much output"), "{cap_error}");
        assert!(started.elapsed() < Duration::from_secs(6));
        drop(pid_send);
        for pid in pid_receive {
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn split_panel_settles_and_loading_redraw_invalidates_old_candidate() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let split = directory.path().join("claude-split");
        std::fs::write(
            &split,
            "#!/bin/sh\nprintf '>\\n'\nIFS= read -r command\nprintf 'Current session 21%% used\\n'\nsleep 0.7\nprintf 'Current week (all models) 54%% used\\n'\nwhile :; do :; done\n",
        )
        .unwrap();
        let loading = directory.path().join("claude-loading");
        std::fs::write(
            &loading,
            "#!/bin/sh\nprintf '>\\n'\nIFS= read -r command\nprintf 'Current session 21%% used\\n'\nsleep 0.1\nprintf '\\033[2JLoading usage data…\\n'\nwhile :; do :; done\n",
        )
        .unwrap();
        for binary in [&split, &loading] {
            let mut permissions = std::fs::metadata(binary).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(binary, permissions).unwrap();
        }
        let usage = capture(&split, directory.path(), Duration::from_secs(4), 16 * 1024).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[1].used_percent.value, Some(54.0));
        assert!(capture(
            &loading,
            directory.path(),
            Duration::from_millis(600),
            16 * 1024
        )
        .unwrap_err()
        .contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_fence_cancels_and_reaps_an_active_capture() {
        use std::{os::unix::fs::PermissionsExt, sync::Arc};
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("claude-shutdown");
        std::fs::write(
            &binary,
            "#!/bin/sh\nprintf '>\\n'\nIFS= read -r command\nwhile :; do :; done\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();
        let registry = Arc::new(ProbeRegistry::default());
        let worker_registry = Arc::clone(&registry);
        let worker_binary = binary.clone();
        let worker_directory = directory.path().to_path_buf();
        let (pid_send, pid_receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            capture_with_registry(
                &worker_binary,
                &worker_directory,
                Duration::from_secs(10),
                16 * 1024,
                &worker_registry,
                Some(&pid_send),
            )
        });
        let pid = pid_receive.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        registry.shutdown();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(worker.join().unwrap().is_err());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert!(registry.begin_launch().is_err());
    }
}
