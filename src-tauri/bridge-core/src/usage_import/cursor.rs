//! Cursor keeps `~/.cursor/chats/**/store.db` and
//! `~/.cursor/ai-tracking/ai-code-tracking.db`. Neither holds a token count,
//! so the source is discovered and reported honestly as unsupported; nothing
//! is imported and no number is invented.

use super::{location_fingerprint, SourceEnv};
use crate::analytics::{CoverageState, DiscoveredAnalyticsSource, ImporterCapability};

pub const AGENT: &str = "cursor";
pub const PROVIDER: &str = "cursor";
pub const UNSUPPORTED_REASON: &str =
    "Cursor's local stores (chats/store.db, ai-tracking/ai-code-tracking.db) record no token counts";

pub fn discover(env: &SourceEnv) -> DiscoveredAnalyticsSource {
    let location = env.cursor_dir();
    let installed = location.join("chats").is_dir() || location.join("ai-tracking").is_dir();
    let reason = if installed {
        UNSUPPORTED_REASON.to_string()
    } else {
        format!("{UNSUPPORTED_REASON}; no Cursor data directory found")
    };
    DiscoveredAnalyticsSource {
        agent: AGENT.into(),
        provider: PROVIDER.into(),
        location_fingerprint: location_fingerprint(AGENT, &location),
        location,
        detected_version: None,
        capability: ImporterCapability::Unsupported,
        coverage: CoverageState::Unsupported,
        reason: Some(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_unsupported_whether_or_not_it_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let env = SourceEnv::for_home(dir.path());
        let absent = discover(&env);
        assert_eq!(absent.capability, ImporterCapability::Unsupported);
        assert_eq!(absent.coverage, CoverageState::Unsupported);
        assert!(absent
            .reason
            .as_deref()
            .unwrap()
            .contains("no token counts"));

        std::fs::create_dir_all(dir.path().join(".cursor/chats/abc")).unwrap();
        std::fs::write(dir.path().join(".cursor/chats/abc/store.db"), b"sqlite").unwrap();
        let present = discover(&env);
        assert_eq!(present.capability, ImporterCapability::Unsupported);
        assert_eq!(present.reason.as_deref(), Some(UNSUPPORTED_REASON));
        present.validate().unwrap();
    }
}
