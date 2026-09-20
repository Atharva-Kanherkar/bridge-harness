//! Only this module persists provider sessions. Values never enter usage snapshots.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn read_file(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|_| "Provider credentials are unavailable".to_string())?;
    let mut content = String::new();
    file.take(65_537)
        .read_to_string(&mut content)
        .map_err(|_| "Invalid provider credentials".to_string())?;
    if content.len() > 65_536 {
        return Err("Provider credentials are too large".into());
    }
    Ok(content)
}

#[cfg(target_os = "macos")]
pub(crate) fn keychain(service: &str, account: Option<&str>) -> Result<Vec<u8>, String> {
    let helper = keychain_helper_executable()?;
    bounded_keychain_helper(&helper, service, account, Duration::from_secs(2), 65_536)
}

#[cfg(target_os = "macos")]
fn keychain_helper_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|_| "Provider session helper is unavailable")?;
    if current.file_name().and_then(|v| v.to_str()) == Some("bridged") {
        return Ok(current);
    }
    let sibling = current.with_file_name("bridged");
    sibling
        .is_file()
        .then_some(sibling)
        .ok_or_else(|| "Provider session helper is unavailable".into())
}

#[cfg(target_os = "macos")]
fn bounded_keychain_helper(
    executable: &Path,
    service: &str,
    account: Option<&str>,
    timeout: Duration,
    cap: usize,
) -> Result<Vec<u8>, String> {
    bounded_keychain_helper_with_mode(executable, service, account, timeout, cap, false)
}

#[cfg(target_os = "macos")]
fn bounded_keychain_helper_with_mode(
    executable: &Path,
    service: &str,
    account: Option<&str>,
    timeout: Duration,
    cap: usize,
    interactive: bool,
) -> Result<Vec<u8>, String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(executable);
    command
        .arg(if interactive {
            "--bridge-keychain-read-interactive"
        } else {
            "--bridge-keychain-read"
        })
        .arg(service);
    if let Some(account) = account {
        command.arg(account);
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Provider session helper is unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Provider session helper is unavailable")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut stored = Vec::new();
        let mut total = 0usize;
        let mut buffer = [0u8; 8192];
        let result = loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break Ok((stored, total)),
                Ok(count) => {
                    total = total.saturating_add(count);
                    let remaining = cap.saturating_add(1).saturating_sub(stored.len());
                    stored.extend_from_slice(&buffer[..count.min(remaining)]);
                }
                Err(_) => break Err(()),
            }
        };
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut output = None;
    while Instant::now() < deadline {
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(_) => {
                    terminate_helper(&mut child);
                    return Err("Provider session helper failed".into());
                }
            }
        }
        if output.is_none() {
            output = receiver.try_recv().ok();
        }
        if status.is_some() && output.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (Some(status), Some(output)) = (status, output) else {
        terminate_helper(&mut child);
        return Err("Provider session read timed out".into());
    };
    let (bytes, total) = output.map_err(|_| "Provider session helper failed")?;
    if !status.success() {
        return Err("Provider session is unavailable in Keychain. Reconnect the provider.".into());
    }
    if total > cap {
        return Err("Provider credentials are too large".into());
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
fn terminate_helper(child: &mut std::process::Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "macos")]
pub fn keychain_helper_read(
    service: &str,
    account: Option<&str>,
    allow_interaction: bool,
) -> Result<Vec<u8>, String> {
    if !allow_interaction {
        let status =
            unsafe { security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0) };
        if status != 0 {
            return Err("Could not disable Keychain interaction".into());
        }
    }
    raw_keychain(service, account, allow_interaction)
}

#[cfg(target_os = "macos")]
fn raw_keychain(
    service: &str,
    account: Option<&str>,
    allow_interaction: bool,
) -> Result<Vec<u8>, String> {
    use core_foundation::{base::TCFType, string::CFString};
    use security_framework::passwords::{generic_password, PasswordOptions};
    use security_framework_sys::item::{
        kSecAttrAccount, kSecUseAuthenticationUI, kSecUseAuthenticationUISkip,
    };
    let mut options = PasswordOptions::new_generic_password(service, account.unwrap_or(""));
    // Background refresh must never raise a Keychain authentication dialog.
    #[allow(deprecated)]
    unsafe {
        if account.is_none() {
            options
                .query
                .retain(|(key, _)| *key != CFString::wrap_under_get_rule(kSecAttrAccount));
        }
        if !allow_interaction {
            options.query.push((
                CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
                CFString::wrap_under_get_rule(kSecUseAuthenticationUISkip).into_CFType(),
            ));
        }
    }
    generic_password(options)
        .map(|v| v.to_vec())
        .map_err(|_| "Provider session is unavailable in Keychain. Reconnect the provider.".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn keychain(_: &str, _: Option<&str>) -> Result<Vec<u8>, String> {
    Err("Provider session requires macOS Keychain".into())
}
#[cfg(not(target_os = "macos"))]
pub fn keychain_helper_read(_: &str, _: Option<&str>, _: bool) -> Result<Vec<u8>, String> {
    Err("Provider session requires macOS Keychain".into())
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenCodeSession {
    pub cookie: String,
    pub workspace: String,
}
const SERVICE: &str = "dev.bridge.deck.provider-usage";

pub fn valid_workspace(value: &str) -> bool {
    value.strip_prefix("wrk_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 128
            && suffix.bytes().all(|v| v.is_ascii_alphanumeric())
    })
}

pub fn save_opencode_session(cookie: &str, workspace: &str) -> Result<(), String> {
    if !valid_workspace(workspace) || !valid_cookie(cookie) {
        return Err("Invalid OpenCode session".into());
    }
    let bytes = serde_json::to_vec(&OpenCodeSession {
        cookie: cookie.into(),
        workspace: workspace.into(),
    })
    .map_err(|_| "Invalid OpenCode session")?;
    #[cfg(target_os = "macos")]
    return security_framework::passwords::set_generic_password(SERVICE, "opencode", &bytes)
        .map_err(|_| "Could not save OpenCode session in Keychain".into());
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err("Connecting OpenCode requires macOS Keychain".into())
    }
}

pub(crate) fn opencode_session() -> Result<OpenCodeSession, String> {
    let bytes = keychain(SERVICE, Some("opencode")).map_err(|_| "Connect OpenCode in Menu Bar settings to read Zen account limits. Local usage is still available.".to_string())?;
    let session: OpenCodeSession = serde_json::from_slice(&bytes)
        .map_err(|_| "Reconnect OpenCode in Menu Bar settings".to_string())?;
    if !valid_workspace(&session.workspace) || !valid_cookie(&session.cookie) {
        return Err("Reconnect OpenCode in Menu Bar settings".into());
    }
    Ok(session)
}
fn valid_cookie(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 16_384
        && value.split(';').all(|part| {
            let Some((name, token)) = part.trim().split_once('=') else {
                return false;
            };
            ["auth", "__Host-auth"].contains(&name)
                && !token.is_empty()
                && token
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && b != b';' && b != b',')
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(target_os = "macos")]
    fn helper(script: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }
    #[test]
    fn cookie_and_workspace_are_scoped() {
        assert!(valid_cookie("auth=abc; __Host-auth=def"));
        assert!(!valid_cookie("auth=a\r\nx=bad"));
        assert!(!valid_cookie("other=abc"));
        assert!(valid_workspace("wrk_ABC123"));
        assert!(!valid_workspace("wrk_a/../../other"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bounded_helper_returns_only_successful_capped_stdout() {
        let (_dir, path) = helper("printf credential");
        assert_eq!(
            bounded_keychain_helper(&path, "service", None, Duration::from_secs(5), 64).unwrap(),
            b"credential"
        );
        let (_dir, path) = helper("printf 12345");
        assert_eq!(
            bounded_keychain_helper(&path, "service", None, Duration::from_secs(5), 4).unwrap_err(),
            "Provider credentials are too large"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bounded_helper_kills_and_reaps_a_timeout() {
        let (_dir, path) = helper("/bin/sleep 5");
        let started = Instant::now();
        assert_eq!(
            bounded_keychain_helper(&path, "service", None, Duration::from_millis(50), 64)
                .unwrap_err(),
            "Provider session read timed out"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bounded_helper_kills_a_descendant_that_keeps_stdout_open() {
        let (_dir, path) = helper("/bin/sleep 5 & exit 0");
        let started = Instant::now();
        assert_eq!(
            bounded_keychain_helper(&path, "service", None, Duration::from_millis(50), 64)
                .unwrap_err(),
            "Provider session read timed out"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
