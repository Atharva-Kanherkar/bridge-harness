use super::{window, AccountUsage};
use chrono::{
    Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Utc,
};
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
static RESET_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bresets?\b\s*:?[\s]*(.+)$").expect("valid reset regex")
});
static RESET_ZONE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\(([^)]+)\)\s*$").expect("valid reset timezone regex"));
static RESET_AT: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)\s+at\s+").expect("valid reset separator regex"));
static RESET_MONTH_GAP: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)\b([a-z]{3})(\d)").expect("valid reset month regex"));
static RESET_HOUR: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(^|\s)(\d{1,2})(\s*(?:am|pm))?$").expect("valid reset hour regex")
});

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
    parse_at(text, Utc::now().timestamp())
}

fn parse_at(text: &str, now: i64) -> Result<AccountUsage, ParseError> {
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
    let session = quota_in_latest_section(
        &clean,
        &["current session", "five hour", "5-hour"],
        &["current week", "weekly"],
        now,
        300,
    );
    let weekly = weekly_quota(&clean, now);
    let scoped = scoped_weekly_windows(&clean, now);
    if session.is_none() && weekly.is_none() && scoped.is_empty() {
        return if lower.contains("loading") || lower.contains("/usage") {
            Err(ParseError::Loading)
        } else {
            Err(ParseError::Malformed)
        };
    }
    let mut usage = AccountUsage {
        observed_at: now,
        source: Some("Claude CLI".into()),
        ..Default::default()
    };
    if let Some((percent, resets_at)) = session {
        usage.windows.push(window(
            "session",
            "5-hour",
            Some(percent),
            resets_at,
            Some(300),
        ));
    }
    if let Some((percent, resets_at)) = weekly {
        usage.windows.push(window(
            "weekly",
            "Weekly",
            Some(percent),
            resets_at,
            Some(10080),
        ));
    }
    usage.windows.extend(scoped);
    Ok(usage)
}

fn scoped_weekly_windows(text: &str, now: i64) -> Vec<bridge_protocol::messages::UsageQuotaWindow> {
    let lines: Vec<&str> = text.lines().collect();
    let label = regex::Regex::new(r"(?i)current\s*week\s*\(([^)]+)\)")
        .expect("valid scoped weekly label regex");
    let percent = regex::Regex::new(
        r"(?i)([0-9]{1,3}(?:\.[0-9]+)?)\s*%\s*(used|spent|consumed|left|remaining|available)",
    )
    .expect("valid percent regex");
    let mut windows: Vec<bridge_protocol::messages::UsageQuotaWindow> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let Some(captures) = label.captures(line) else {
            continue;
        };
        let raw_model = captures
            .get(1)
            .map(|value| value.as_str().trim())
            .unwrap_or("");
        // Reject hostile or ambiguous labels rather than truncating two long
        // names into the same persisted-looking id.
        if raw_model.chars().count() > 160 || raw_model.chars().any(char::is_control) {
            continue;
        }
        let model = raw_model;
        let normalized = normalized_model(model);
        if normalized.is_empty() || is_all_models(&normalized) {
            continue;
        }
        let id = format!("weekly-scoped-{}", slug(model));
        // The newest occurrence owns this model even when it is incomplete: a
        // redraw replacing `12% used` with `loading` removes the stale value.
        let existing_index = windows.iter().position(|candidate| candidate.id == id);

        // Claude redraws /usage incrementally. Keep each scoped percentage inside
        // its own label section so an all-model or sibling-model value can never
        // be borrowed while the next row is still arriving.
        let section: Vec<&str> = lines[index..]
            .iter()
            .take(14)
            .enumerate()
            .take_while(|(offset, candidate)| *offset == 0 || !is_usage_section_boundary(candidate))
            .map(|(_, candidate)| *candidate)
            .collect();
        let value = section.iter().find_map(|candidate| {
            let captures = percent.captures(candidate)?;
            let raw = captures.get(1)?.as_str().parse::<f64>().ok()?;
            if !raw.is_finite() || !(0.0..=100.0).contains(&raw) {
                return None;
            }
            let direction = captures.get(2)?.as_str().to_ascii_lowercase();
            Some(
                if matches!(direction.as_str(), "left" | "remaining" | "available") {
                    100.0 - raw
                } else {
                    raw
                },
            )
        });
        let Some(value) = value else {
            if let Some(existing_index) = existing_index {
                windows.remove(existing_index);
            }
            continue;
        };
        let title = if normalized.ends_with("only") {
            format!("Weekly · {model}")
        } else {
            format!("Weekly · {model} only")
        };
        let resets_at = reset_in_lines(&section, now, 10080);
        let parsed = window(&id, &title, Some(value), resets_at, Some(10080));
        if let Some(existing_index) = existing_index {
            windows[existing_index] = parsed;
        } else if windows.len() < 32 {
            windows.push(parsed);
        }
    }
    windows
}

fn is_usage_section_boundary(line: &str) -> bool {
    let normalized = line
        .trim_start()
        .trim_start_matches(|character: char| !character.is_alphanumeric())
        .to_ascii_lowercase()
        .replace(['-', '_'], " ");
    normalized.starts_with("current ")
        || normalized.starts_with("weekly")
        || normalized.starts_with("five hour")
        || normalized.starts_with("5 hour")
        || normalized.starts_with("extra usage")
}

fn normalized_model(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_all_models(value: &str) -> bool {
    value == "allmodels"
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    let mut dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            result.push(character);
            dash = false;
        } else if !dash && !result.is_empty() {
            result.push('-');
            dash = true;
        }
    }
    result.trim_end_matches('-').to_string()
}

fn quota_in_latest_section(
    text: &str,
    labels: &[&str],
    other_labels: &[&str],
    now: i64,
    window_minutes: i64,
) -> Option<(f64, Option<i64>)> {
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
            let section = &text[index..index + boundary];
            let capture = regex::Regex::new(
                r"(?i)([0-9]{1,3}(?:\.[0-9]+)?)\s*%\s*(used|spent|consumed|left|remaining|available)",
            )
                .expect("valid percent regex")
                .captures(section)?;
            let raw = capture.get(1)?.as_str().parse::<f64>().ok()?;
            let direction = capture.get(2)?.as_str().to_ascii_lowercase();
            let used = if matches!(direction.as_str(), "left" | "remaining" | "available") {
                100.0 - raw
            } else {
                raw
            };
            let lines: Vec<&str> = section.lines().take(14).collect();
            Some((used, reset_in_lines(&lines, now, window_minutes)))
        })
        .filter(|(value, _)| value.is_finite() && (0.0..=100.0).contains(value))
}

fn weekly_quota(text: &str, now: i64) -> Option<(f64, Option<i64>)> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("current week (all models)") {
        quota_in_latest_section(
            text,
            &["current week (all models)"],
            &["current week", "current session", "five hour", "5-hour"],
            now,
            10080,
        )
    } else {
        quota_in_latest_section(
            text,
            &["weekly"],
            &["current week", "current session", "five hour", "5-hour"],
            now,
            10080,
        )
    }
}

fn reset_in_lines(lines: &[&str], now: i64, window_minutes: i64) -> Option<i64> {
    lines
        .iter()
        .find_map(|line| parse_reset(line, now, window_minutes))
}

fn parse_reset(line: &str, now: i64, window_minutes: i64) -> Option<i64> {
    let matched = RESET_LINE.captures(line)?;
    let mut raw = matched.get(1)?.as_str().trim().to_string();
    let zone = if let Some(capture) = RESET_ZONE.captures(&raw) {
        let zone = capture.get(1)?.as_str().parse::<chrono_tz::Tz>().ok()?;
        raw.truncate(capture.get(0)?.start());
        zone
    } else {
        iana_time_zone::get_timezone()
            .ok()?
            .parse::<chrono_tz::Tz>()
            .ok()?
    };
    // CodexBar's reset parser accepts the CLI's comma, "at", and clock
    // spacing variants. Normalize punctuation before applying date formats.
    raw = RESET_AT.replace_all(&raw, " ").replace(',', " ");
    raw = RESET_MONTH_GAP.replace_all(&raw, "${1} ${2}").into_owned();
    raw = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    raw = RESET_HOUR.replace(&raw, "${1}${2}:00${3}").into_owned();
    let now = Utc.timestamp_opt(now, 0).single()?;
    let local_now = now.with_timezone(&zone);
    let explicit = ["%b %e %Y %I:%M%p", "%b %e %Y %I:%M %p", "%b %e %Y %H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(&raw, format).ok())
        .and_then(|value| local_datetime(zone, value));
    if let Some(explicit) = explicit {
        return choose_occurrence(vec![explicit], now, window_minutes)
            .map(|value| value.timestamp());
    }
    let dated_input = format!("2000 {raw}");
    let dated = ["%Y %b %e %I:%M%p", "%Y %b %e %I:%M %p", "%Y %b %e %H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(&dated_input, format).ok())
        .and_then(|value| {
            let mut candidates = Vec::new();
            for year in (local_now.year() - 1)..=(local_now.year() + 1) {
                if let Some(date) = NaiveDate::from_ymd_opt(year, value.month(), value.day()) {
                    if let Some(candidate) = local_datetime(zone, date.and_time(value.time())) {
                        candidates.push(candidate);
                    }
                }
            }
            choose_occurrence(candidates, now, window_minutes)
        });
    if dated.is_some() {
        return dated.map(|value| value.timestamp());
    }
    let time = ["%I:%M%p", "%I:%M %p", "%I%p", "%I %p", "%H:%M"]
        .iter()
        .find_map(|format| NaiveTime::parse_from_str(&raw, format).ok())?;
    let mut candidates = Vec::new();
    for day in -1..=1 {
        let date = local_now.date_naive() + ChronoDuration::days(day);
        if let Some(candidate) = local_datetime(zone, date.and_time(time)) {
            candidates.push(candidate);
        }
    }
    choose_occurrence(candidates, now, window_minutes).map(|value| value.timestamp())
}

fn local_datetime(zone: chrono_tz::Tz, value: NaiveDateTime) -> Option<chrono::DateTime<Utc>> {
    match zone.from_local_datetime(&value) {
        LocalResult::Single(value) | LocalResult::Ambiguous(value, _) => {
            Some(value.with_timezone(&Utc))
        }
        LocalResult::None => None,
    }
}

fn choose_occurrence(
    mut candidates: Vec<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
    window_minutes: i64,
) -> Option<chrono::DateTime<Utc>> {
    candidates.sort();
    let future = candidates.iter().copied().find(|value| *value >= now);
    let past = candidates.iter().copied().rev().find(|value| *value < now);
    let horizon = ChronoDuration::minutes(window_minutes);
    future
        .filter(|value| *value - now <= horizon)
        .or_else(|| past.filter(|value| now - *value <= horizon))
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
    fn parses_directional_percentages_and_section_local_resets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-13T06:00:00Z")
            .unwrap()
            .timestamp();
        let usage = parse_at(
            "Current session\n100% remaining\nResets 2:48pm (Asia/Kolkata)\n\
             Current week (all models)\n90% remaining\nResets Sep 18 at 11:30am (Asia/Kolkata)\n\
             Current week (Fable)\n89% remaining\nResets Sep 18 at 11:30am (Asia/Kolkata)",
            now,
        )
        .unwrap();
        assert_eq!(
            parse_reset("Resets 2:48pm (Asia/Kolkata)", now, 300),
            Some(1_789_291_080)
        );
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(usage.windows[0].resets_at, Some(1_789_291_080));
        assert_eq!(usage.windows[1].used_percent.value, Some(10.0));
        assert_eq!(usage.windows[1].resets_at, Some(1_789_711_200));
        assert_eq!(usage.windows[2].used_percent.value, Some(11.0));
        assert_eq!(usage.windows[2].resets_at, usage.windows[1].resets_at);
    }

    #[test]
    fn malformed_and_redrawn_resets_are_not_borrowed() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-13T06:00:00Z")
            .unwrap()
            .timestamp();
        let usage = parse_at(
            "Current session 12% used\nResets 1:00pm (Asia/Kolkata)\n\
             Current week (all models) 20% used\nResets nonsense\x1b[2J\
             Current session 0% used\nResets 2:48pm (Asia/Kolkata)\n\
             Current week (all models) 10% used\nResets malformed\n\
             Current week (Fable) 11% used\nResets Sep 18 at 11:30am (Asia/Kolkata)",
            now,
        )
        .unwrap();
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(usage.windows[0].resets_at, Some(1_789_291_080));
        assert_eq!(usage.windows[1].used_percent.value, Some(10.0));
        assert_eq!(usage.windows[1].resets_at, None);
        assert_eq!(usage.windows[2].used_percent.value, Some(11.0));
        assert!(usage.windows[2].resets_at.is_some());
    }

    #[test]
    fn reset_timezone_rollover_and_horizon_are_strict() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-12-31T22:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            parse_reset("Resets Jan 1 at 4:00am (Asia/Kolkata)", now, 10080),
            Some(1_798_756_200)
        );
        assert_eq!(parse_reset("Resets 4:00am (Unknown/Zone)", now, 300), None);
        assert_eq!(
            parse_reset("Resets Dec 1 at 4:00am (Asia/Kolkata)", now, 10080),
            None
        );
        assert_eq!(
            parse_reset("Resets Jan 1, 2028, 4:00am (Asia/Kolkata)", now, 10080),
            None
        );
    }

    #[test]
    fn reset_dates_accept_claude_cli_punctuation_and_clock_variants() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-13T06:00:00Z")
            .unwrap().timestamp();
        let expected = chrono::DateTime::parse_from_rfc3339("2026-09-18T06:00:00Z")
            .unwrap().timestamp();
        for label in [
            "Sep 18, 11:30 am", "Sep 18 AT 11:30am", "Sep18,11:30AM",
            "Sep 18, 2026, 11:30am", "Sep 18 2026 11:30 AM", "Sep 18 11:30",
        ] {
            assert_eq!(parse_reset(&format!("Resets {label} (Asia/Kolkata)"), now, 10080),
                Some(expected), "{label}");
        }
        let hour = chrono::DateTime::parse_from_rfc3339("2026-09-18T05:30:00Z")
            .unwrap().timestamp();
        for label in ["Sep 18, 11am", "Sep 18 2026 11 AM", "Sep 18, 11"] {
            assert_eq!(parse_reset(&format!("Resets {label} (Asia/Kolkata)"), now, 10080),
                Some(hour), "{label}");
        }
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
        assert_eq!(
            parse("Current session 81% remaining").unwrap().windows[0]
                .used_percent
                .value,
            Some(19.0)
        );
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
        assert_eq!(scoped_after_total.windows.len(), 2);
        assert_eq!(scoped_after_total.windows[0].used_percent.value, Some(42.0));
        assert_eq!(scoped_after_total.windows[1].id, "weekly-scoped-sonnet");
        assert_eq!(scoped_after_total.windows[1].used_percent.value, Some(7.0));
        let fable_after_total = parse(
            "Current session 3% used\nCurrent week (all models) 42% used\nCurrent week (Fable)\n7.5% used",
        )
        .unwrap();
        assert_eq!(fable_after_total.windows.len(), 3);
        assert_eq!(fable_after_total.windows[1].used_percent.value, Some(42.0));
        assert_eq!(fable_after_total.windows[2].id, "weekly-scoped-fable");
        assert_eq!(fable_after_total.windows[2].label, "Weekly · Fable only");
        assert_eq!(fable_after_total.windows[2].used_percent.value, Some(7.5));
        assert!(fable_after_total.windows[2].resets_at.is_none());
        let incomplete_fable = parse(
            "Current week (all models) 42% used\nCurrent week (Fable) loading\nCurrent week (Iris) 8% used",
        )
        .unwrap();
        assert_eq!(incomplete_fable.windows.len(), 2);
        assert_eq!(incomplete_fable.windows[1].id, "weekly-scoped-iris");
        assert_eq!(incomplete_fable.windows[1].used_percent.value, Some(8.0));
        let long_unicode = format!("Current session {}✨ 9% used", "x".repeat(790));
        let _ = parse(&long_unicode);
    }

    #[test]
    fn latest_redraw_owns_scoped_rows_and_unknown_values_stay_absent() {
        let usage = parse(
            "Current week (all models) 11% used\nCurrent week (Fable) 12% used\x1b[2JCurrent week (all models) 21% used\nCurrent week (Fable) --\nCurrent week (Iris only) 4.25% used",
        )
        .unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].used_percent.value, Some(21.0));
        assert_eq!(usage.windows[1].id, "weekly-scoped-iris-only");
        assert_eq!(usage.windows[1].label, "Weekly · Iris only");
        assert_eq!(usage.windows[1].used_percent.value, Some(4.25));
    }

    #[test]
    fn scoped_sections_stop_at_every_usage_boundary_and_latest_unknown_removes_old_value() {
        let bounded = parse(
            "Current week (all models) 21% used\nCurrent week (Fable) loading\nCurrent session 12% used",
        )
        .unwrap();
        assert_eq!(bounded.windows.len(), 2);
        assert!(bounded
            .windows
            .iter()
            .all(|window| window.id != "weekly-scoped-fable"));

        let duplicate = parse(
            "Current week (all models) 21% used\nCurrent week (Fable) 12% used\nCurrent week (Fable) loading",
        )
        .unwrap();
        assert_eq!(duplicate.windows.len(), 1);
        assert_eq!(duplicate.windows[0].id, "weekly");
    }

    #[test]
    fn scoped_only_panels_include_named_models_and_stay_bounded() {
        let usage = parse("Current week (Fable) 7.5% used\nCurrent week (Opus) 8% used").unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].id, "weekly-scoped-fable");
        assert_eq!(usage.windows[1].id, "weekly-scoped-opus");

        let long_name = "x".repeat(220);
        let mut panel = format!("Current week ({long_name}) 1% used\n");
        for index in 0..40 {
            panel.push_str(&format!("Current week (Model {index}) 2% used\n"));
        }
        panel.push_str("Current week (Model 0) loading\n");
        let bounded = parse(&panel).unwrap();
        assert_eq!(bounded.windows.len(), 31);
        assert!(bounded
            .windows
            .iter()
            .all(|window| window.id != "weekly-scoped-model-0"));
        assert!(bounded.windows[0].label.chars().count() <= "Weekly ·  only".chars().count() + 160);
    }

    #[cfg(unix)]
    #[test]
    fn fake_cli_is_reaped_after_a_successful_capture() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("claude-fake");
        let pid_file = directory.path().join("pid");
        std::fs::write(&binary, format!(
            "#!/bin/sh\nprintf '%s' $$ > '{}'\nprintf '>\\n'\nIFS= read -r command\nprintf '\\033[2JCurrent session 27%% used\\rCurrent week (all models) 63%% used\\rCurrent week (Fable) 6.5%% used\\n'\nwhile :; do :; done\n",
            pid_file.display()
        )).unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary, permissions).unwrap();

        let usage = capture(&binary, directory.path(), Duration::from_secs(4), 16 * 1024).unwrap();
        assert_eq!(usage.windows[0].used_percent.value, Some(27.0));
        assert_eq!(usage.windows[1].used_percent.value, Some(63.0));
        assert_eq!(usage.windows[2].id, "weekly-scoped-fable");
        assert_eq!(usage.windows[2].used_percent.value, Some(6.5));
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
