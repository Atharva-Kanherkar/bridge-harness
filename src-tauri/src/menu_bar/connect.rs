//! OpenCode Go is API-key based; macOS owns the browser sign-in session.
const OPENCODE_AUTH_URL: &str = "https://opencode.ai/auth";

pub(super) fn open_default_browser() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(OPENCODE_AUTH_URL)
            .status()
            .map_err(|_| {
                "Could not open your default browser. Visit opencode.ai/auth to sign in."
                    .to_string()
            })?;
        if status.success() {
            Ok(())
        } else {
            Err("Could not open your default browser. Visit opencode.ai/auth to sign in.".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    Err("OpenCode sign-in is only available in Bridge for macOS.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_first_party_auth_page() {
        assert_eq!(OPENCODE_AUTH_URL, "https://opencode.ai/auth");
    }
}
