//! Structured, no-turn usage probe. Claude Code owns its credentials and renewal.
use super::{public_text, AccountUsage};
use serde_json::{json, Value};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub(super) const UNAVAILABLE: &str = "Claude usage SDK unavailable.";
const TIMEOUT: Duration = Duration::from_secs(35);
const OUTPUT_LIMIT: u64 = 128 * 1024;

pub(super) fn read(core: &crate::BridgeCore) -> Result<AccountUsage, String> {
    let node =
        crate::binary::resolve("node").ok_or_else(|| format!("{UNAVAILABLE} Install Node.js."))?;
    let sidecar = crate::claude_adapter::sidecar_entry().map_err(|_| UNAVAILABLE.to_string())?;
    let binary = crate::binary::resolve("claude");
    let base = core
        .database_path
        .parent()
        .ok_or("Cannot prepare Claude usage probe directory.")?;
    let directory = base.join("claude-usage-probe");
    std::fs::create_dir_all(&directory)
        .map_err(|_| "Cannot prepare Claude usage probe directory.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect Claude usage probe directory.")?;
    }
    let mut command = Command::new(node);
    crate::binary::hydrate_command_path(&mut command);
    crate::claude_adapter::configure_sdk_environment(&mut command);
    command
        .arg(sidecar)
        .arg(json!({"usage":true,"cwd":directory,"executablePath":binary}).to_string())
        .current_dir(&directory)
        .env_remove("NODE_OPTIONS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    capture(&mut command, TIMEOUT)
}

fn capture(command: &mut Command, timeout: Duration) -> Result<AccountUsage, String> {
    let registry = &super::claude_cli::ACTIVE_PROBES;
    let launch = registry.begin_launch()?;
    crate::adapters::configure_process_group(command);
    let mut child = command
        .spawn()
        .map_err(|_| format!("{UNAVAILABLE} Cannot start usage probe."))?;
    let active = registry.register(child.id());
    drop(launch);
    let stdout = child.stdout.take().expect("usage stdout is piped");
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        let result = stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_string(&mut output)
            .map(|_| output);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(timeout);
    // Covers success, timeout, overflow, malformed response and daemon shutdown.
    crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    drop(active);
    let output = result
        .map_err(|_| "Claude Code usage timed out. Try Refresh.")?
        .map_err(|_| "Could not read Claude Code usage response.")?;
    if output.len() as u64 > OUTPUT_LIMIT {
        return Err("Claude Code usage response was too large.".into());
    }
    parse_response(&output)
}

fn parse_response(output: &str) -> Result<AccountUsage, String> {
    let frame: Value = serde_json::from_str(output)
        .map_err(|_| "Claude Code returned an invalid usage response.")?;
    if frame["type"] == "claude_usage_error" {
        return Err(match frame["code"].as_str() {
            Some("unsupported" | "malformed") => {
                format!("{UNAVAILABLE} Update Claude Code to read structured limits.")
            }
            Some("timeout") => "Claude Code usage timed out. Try Refresh.".into(),
            Some("initialization_failed") => {
                "Claude Code usage initialization failed. Open Claude Code and check its sign-in."
                    .into()
            }
            _ => {
                "Claude Code could not read usage limits. Try Refresh or check its sign-in.".into()
            }
        });
    }
    if frame["type"] != "claude_usage" || !frame["rateLimitsAvailable"].is_boolean() {
        return Err("Claude Code returned an invalid usage response.".into());
    }
    if frame["rateLimitsAvailable"] == false {
        return Err("Claude Code is not reporting subscription limits for this sign-in. Check your Claude Code account.".into());
    }
    let mut usage = super::claude::parse(&frame["rateLimits"], chrono::Utc::now().timestamp())?;
    usage.account = public_text(&frame["account"]["email"]);
    usage.plan =
        public_text(&frame["account"]["subscriptionType"]).map(|plan| match plan.as_str() {
            "max" | "claude_max_subscription" => "Max".into(),
            "claude_max_5x_subscription" => "Max (5x)".into(),
            "claude_max_20x_subscription" => "Max (20x)".into(),
            "pro" | "claude_pro_subscription" => "Pro".into(),
            "team" | "claude_teams_subscription" => "Team".into(),
            "enterprise" | "claude_enterprise_subscription" => "Enterprise".into(),
            _ => plan,
        });
    usage.source = Some("Claude Code".into());
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_limits_keep_unknown_distinct_from_zero_and_include_fable() {
        let usage = parse_response(&json!({
            "type":"claude_usage", "rateLimitsAvailable":true,
            "account":{"email":"one@example.test","subscriptionType":"max"},
            "rateLimits": {
                "five_hour":{"utilization":0}, "seven_day":{"utilization":null},
                "model_scoped":[{"display_name":"Fable","utilization":31,"resets_at":"2026-09-25T06:00:00Z"}]
            }
        }).to_string()).unwrap();
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(usage.windows[1].used_percent.value, None);
        assert_eq!(usage.windows[2].label, "Weekly · Fable only");
        assert_eq!(usage.windows[2].used_percent.value, Some(31.0));
        assert_eq!(usage.account.as_deref(), Some("one@example.test"));
        assert_eq!(usage.source.as_deref(), Some("Claude Code"));
    }

    #[test]
    fn unavailable_and_malformed_responses_never_become_zero_usage() {
        for response in [
            json!({}),
            json!({"type":"claude_usage","rateLimitsAvailable":false}),
            json!({"type":"claude_usage","rateLimitsAvailable":true,"rateLimits":{}}),
        ] {
            assert!(parse_response(&response.to_string()).is_err());
        }
        let error = parse_response(
            r#"{"type":"claude_usage_error","code":"unsupported","message":"SECRET"}"#,
        )
        .unwrap_err();
        assert!(error.starts_with(UNAVAILABLE));
        assert!(!error.contains("SECRET"));
    }

    #[test]
    #[cfg(unix)]
    fn hung_probe_process_is_reaped_on_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let mut command = Command::new("sh");
        command
            .args(["-c", "echo $$ > \"$1\"; exec sleep 60", "probe"])
            .arg(&pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        assert!(capture(&mut command, Duration::from_millis(250))
            .unwrap_err()
            .contains("timed out"));
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "probe must not outlive refresh"
        );
    }
}
