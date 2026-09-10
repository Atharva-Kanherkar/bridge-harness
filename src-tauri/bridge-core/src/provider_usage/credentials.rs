//! Only this module persists provider sessions. Values never enter usage snapshots.
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

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
        options.query.push((
            CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
            CFString::wrap_under_get_rule(kSecUseAuthenticationUISkip).into_CFType(),
        ));
    }
    generic_password(options)
        .map(|v| v.to_vec())
        .map_err(|_| "Provider session is unavailable in Keychain. Reconnect the provider.".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn keychain(_: &str, _: Option<&str>) -> Result<Vec<u8>, String> {
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
    #[test]
    fn cookie_and_workspace_are_scoped() {
        assert!(valid_cookie("auth=abc; __Host-auth=def"));
        assert!(!valid_cookie("auth=a\r\nx=bad"));
        assert!(!valid_cookie("other=abc"));
        assert!(valid_workspace("wrk_ABC123"));
        assert!(!valid_workspace("wrk_a/../../other"));
    }
}
