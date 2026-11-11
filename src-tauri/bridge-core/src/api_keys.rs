//! Saved API keys: the ones the person keeps for every chat.
//!
//! The value lives in the macOS Keychain under its own service, one item per
//! key name, and never in the database. The name, the kind and the sites the
//! key may be sent to live in `configuration_entries` (kind `api_keys`), so
//! the settings page and the agent's instructions can list them without a
//! Keychain read. The credential broker is the only reader of the value.

use crate::{account_secrets::keychain, BridgeError};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The Keychain service saved keys live under. `bridged`'s Keychain read
/// helper only reads services it names, and this is one of them.
pub const KEYCHAIN_SERVICE: &str = "dev.bridge.deck.api-keys";
const SERVICE: &str = KEYCHAIN_SERVICE;
const ENTRY_KIND: &str = "api_keys";
const MAX_NAME_BYTES: usize = 64;
const MAX_HOSTS: usize = 20;

/// What Bridge records about a saved key besides its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub kind: String,
    pub hosts: Vec<String>,
    /// Chat placeholders (`sec_…`) this key was saved from. They keep
    /// resolving after a restart, so an old chat's placeholder still works.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// Whether this build can keep a key past a restart.
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

/// A key name reads like an environment variable: `OPENAI_API_KEY`. Upper case
/// keeps it apart from the random `sec_…` placeholders a pasted key gets.
pub fn validate_name(name: &str) -> Result<String, BridgeError> {
    let name = name.trim();
    let mut chars = name.chars();
    let valid = name.len() <= MAX_NAME_BYTES
        && chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !valid || name.len() < 2 {
        return Err(BridgeError::Invalid(
            "Name the key in capitals, digits and underscores, like OPENAI_API_KEY".into(),
        ));
    }
    Ok(name.to_owned())
}

/// A site a key may be sent to, as the person typed it: `api.github.com`,
/// `https://api.github.com/user` or `*.example.com`. Returns the bare host.
pub fn normalize_host(input: &str) -> Result<String, BridgeError> {
    let trimmed = input.trim().to_ascii_lowercase();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(&trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (name, Some(port)),
        _ => (host, None),
    };
    let bare = name.strip_prefix("*.").unwrap_or(name);
    let valid = !bare.is_empty()
        && bare.len() <= 253
        && port.is_none_or(|port| port.len() <= 5)
        && bare.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !valid {
        return Err(BridgeError::Invalid(format!(
            "\"{}\" is not a site Bridge can send a key to. Use a host like api.example.com",
            input.trim()
        )));
    }
    Ok(host.to_owned())
}

/// Normalize, de-duplicate and bound a list of sites.
pub fn normalize_hosts(hosts: &[String]) -> Result<Vec<String>, BridgeError> {
    let mut normalized: Vec<String> = Vec::new();
    for host in hosts.iter().filter(|host| !host.trim().is_empty()) {
        let host = normalize_host(host)?;
        if !normalized.contains(&host) {
            normalized.push(host);
        }
    }
    if normalized.len() > MAX_HOSTS {
        return Err(BridgeError::Invalid(format!(
            "A key can be allowed on at most {MAX_HOSTS} sites"
        )));
    }
    Ok(normalized)
}

pub fn validate_value(value: &str) -> Result<Zeroizing<String>, BridgeError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(BridgeError::Invalid("The key cannot be empty".into()));
    }
    if value.len() > crate::account_secrets::MAX_KEY_BYTES || value.chars().any(char::is_control) {
        return Err(BridgeError::Invalid(
            "That does not look like an API key. Paste the key on one line".into(),
        ));
    }
    Ok(Zeroizing::new(value.to_owned()))
}

pub fn list(db: &Connection) -> Result<Vec<(String, Metadata)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, payload FROM configuration_entries WHERE kind=?1 ORDER BY id",
    )?;
    let rows = statement
        .query_map(params![ENTRY_KIND], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    // A row this build cannot read is skipped rather than failing the list:
    // one bad entry must not hide every other key.
    Ok(rows
        .into_iter()
        .filter_map(|(name, payload)| {
            serde_json::from_str::<Metadata>(&payload)
                .ok()
                .map(|metadata| (name, metadata))
        })
        .collect())
}

pub fn put(db: &Connection, name: &str, metadata: &Metadata) -> Result<(), BridgeError> {
    let payload =
        serde_json::to_string(metadata).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let now = chrono::Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![ENTRY_KIND, name, payload, now],
    )?;
    Ok(())
}

pub fn remove(db: &Connection, name: &str) -> Result<(), BridgeError> {
    db.execute(
        "DELETE FROM configuration_entries WHERE kind=?1 AND id=?2",
        params![ENTRY_KIND, name],
    )?;
    Ok(())
}

/// Write the value as a new Keychain item. Updating an existing item would
/// keep its access list, which names the build that first wrote it, so a key
/// pasted again after a rebuild would still be unreadable on the next launch.
pub fn save_value(name: &str, value: &str) -> Result<(), BridgeError> {
    keychain::delete(SERVICE, name)?;
    keychain::save(SERVICE, name, value)
}

pub fn read_value(name: &str) -> Result<Option<Zeroizing<String>>, BridgeError> {
    keychain::read(SERVICE, name)
}

pub fn delete_value(name: &str) -> Result<(), BridgeError> {
    keychain::delete(SERVICE, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_environment_variables() {
        assert_eq!(validate_name(" GITHUB_TOKEN ").unwrap(), "GITHUB_TOKEN");
        assert_eq!(validate_name("OPENAI_API_KEY_2").unwrap(), "OPENAI_API_KEY_2");
        for bad in ["", "A", "github_token", "sec_abc", "1PASSWORD", "MY KEY", "KEY-1"] {
            assert!(validate_name(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn sites_accept_hosts_urls_and_wildcards() {
        assert_eq!(normalize_host("api.github.com").unwrap(), "api.github.com");
        assert_eq!(normalize_host(" https://API.GitHub.com/user?x=1 ").unwrap(), "api.github.com");
        assert_eq!(normalize_host("*.example.com").unwrap(), "*.example.com");
        assert_eq!(normalize_host("localhost:8080").unwrap(), "localhost:8080");
        for bad in ["", "https://", "exa mple.com", "user@evil.com", "-bad.com", "a..b"] {
            assert!(normalize_host(bad).is_err(), "{bad} should be refused");
        }
        let hosts = normalize_hosts(&["api.x.com".into(), "API.X.COM".into(), " ".into()]).unwrap();
        assert_eq!(hosts, vec!["api.x.com"]);
    }

    #[test]
    fn metadata_round_trips_through_configuration_entries() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        let metadata = Metadata {
            kind: "github".into(),
            hosts: vec!["api.github.com".into()],
            aliases: vec!["sec_abc".into()],
        };
        put(&db, "GITHUB_TOKEN", &metadata).unwrap();
        assert_eq!(list(&db).unwrap(), vec![("GITHUB_TOKEN".to_owned(), metadata)]);
        remove(&db, "GITHUB_TOKEN").unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn values_never_go_to_the_database() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        save_value("API_KEYS_TEST_VALUE", "sk-test-canary-value").unwrap();
        put(&db, "API_KEYS_TEST_VALUE", &Metadata { kind: "custom".into(), hosts: vec![], aliases: vec![] }).unwrap();
        let payload: String = db
            .query_row("SELECT payload FROM configuration_entries WHERE kind='api_keys'", [], |row| row.get(0))
            .unwrap();
        assert!(!payload.contains("canary"));
        assert_eq!(read_value("API_KEYS_TEST_VALUE").unwrap().as_deref().map(String::as_str), Some("sk-test-canary-value"));
        delete_value("API_KEYS_TEST_VALUE").unwrap();
        assert_eq!(read_value("API_KEYS_TEST_VALUE").unwrap(), None);
    }
}
