//! OpenCode Go's account usage API; no browser cookies or inference calls.
use super::*;
use rusqlite::OptionalExtension;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

const MAX_AUTH_BYTES: usize = 1_048_576;

/// Bearer tokens to try in order. The environment override and OpenCode 2.x's
/// own store are each authoritative; 1.x's `auth.json` is read only without them.
pub(super) fn credentials() -> Result<Vec<Zeroizing<String>>, String> {
    let content = match std::env::var("OPENCODE_AUTH_CONTENT") {
        Ok(value) if !value.is_empty() => Some(Zeroizing::new(value)),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("OPENCODE_AUTH_CONTENT is not valid text.".into())
        }
        _ => None,
    };
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|base| base.join("opencode"));
    let fallback = std::env::var("OPENCODE_API_KEY").ok().map(Zeroizing::new);
    if content.is_none() {
        if let Some(data) = &data {
            let stored = stored_credentials(
                &data.join("opencode.db"),
                chrono::Utc::now().timestamp_millis(),
            )?;
            if let Some(stored) = stored {
                return Ok(if stored.is_empty() {
                    trimmed(fallback).into_iter().collect()
                } else {
                    stored
                });
            }
        }
    }
    Ok(
        key_from_sources(content, data.map(|data| data.join("auth.json")), fallback)?
            .into_iter()
            .collect(),
    )
}

/// OpenCode 2.x keeps credentials in its database's `credential` table. A Go
/// API key comes first; the signed-in OpenCode account follows, so an expired
/// key still reads usage when OpenCode itself is signed in. `None` means 1.x.
fn stored_credentials(path: &Path, now_ms: i64) -> Result<Option<Vec<Zeroizing<String>>>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let unreadable = "Could not read OpenCode authentication. Check access to its database.";
    let db = crate::usage_import::opencode::open_read_only(path).map_err(|_| unreadable)?;
    let _ = db.busy_timeout(Duration::from_secs(2));
    let has_table = db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='credential'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| unreadable)?
        .is_some();
    if !has_table {
        return Ok(None);
    }
    let mut statement = db
        .prepare("SELECT integration_id, value FROM credential WHERE integration_id IN ('opencode-go','opencode') ORDER BY COALESCE(active,0) DESC, time_updated DESC")
        .map_err(|_| unreadable)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Zeroizing::new(row.get::<_, String>(1)?),
            ))
        })
        .map_err(|_| unreadable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unreadable)?;
    #[derive(Deserialize)]
    struct Stored {
        #[serde(rename = "type")]
        kind: String,
        key: Option<String>,
        access: Option<String>,
        expires: Option<i64>,
        metadata: Option<Metadata>,
    }
    #[derive(Deserialize)]
    struct Metadata {
        server: Option<String>,
    }
    let (mut key, mut account, mut expired) = (None, None, false);
    for (integration, value) in rows {
        let Ok(stored) = serde_json::from_str::<Stored>(&value) else {
            continue;
        };
        if integration == "opencode-go"
            && key.is_none()
            && matches!(stored.kind.as_str(), "key" | "api")
        {
            key = trimmed(stored.key.map(Zeroizing::new));
        } else if integration == "opencode" && account.is_none() && stored.kind == "oauth" {
            // The token goes only to opencode.ai, so a self-hosted console's sign-in is skipped.
            let server = stored.metadata.and_then(|metadata| metadata.server);
            if server.is_some_and(|server| {
                reqwest::Url::parse(&server)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .as_deref()
                    != Some("opencode.ai")
            }) {
                continue;
            }
            if stored.expires.is_some_and(|expires| expires <= now_ms) {
                expired = true;
                continue;
            }
            account = trimmed(stored.access.map(Zeroizing::new));
        }
    }
    let found = key.into_iter().chain(account).collect::<Vec<_>>();
    if found.is_empty() && expired {
        return Err(
            "Your OpenCode sign-in has expired. Open OpenCode to renew it, then refresh usage."
                .into(),
        );
    }
    Ok(Some(found))
}

fn trimmed(value: Option<Zeroizing<String>>) -> Option<Zeroizing<String>> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| Zeroizing::new(value.trim().to_string()))
}

fn key_from_sources(
    content: Option<Zeroizing<String>>,
    path: Option<PathBuf>,
    fallback: Option<Zeroizing<String>>,
) -> Result<Option<Zeroizing<String>>, String> {
    let contents = match content {
        Some(contents) => contents,
        None => {
            match path {
                Some(path) => match std::fs::File::open(path) {
                    Ok(file) => {
                        let mut bytes = Zeroizing::new(Vec::new());
                        file.take((MAX_AUTH_BYTES + 1) as u64).read_to_end(&mut bytes)
                        .map_err(|_| "Could not read OpenCode authentication. Sign in with OpenCode again.")?;
                        if bytes.len() > MAX_AUTH_BYTES {
                            return Err("OpenCode authentication file is too large.".into());
                        }
                        Zeroizing::new(
                            String::from_utf8(bytes.to_vec())
                                .map_err(|_| "OpenCode authentication is not valid text.")?,
                        )
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Zeroizing::new("{}".into())
                    }
                    Err(_) => return Err(
                        "Could not read OpenCode authentication. Check access to its auth file."
                            .into(),
                    ),
                },
                None => Zeroizing::new("{}".into()),
            }
        }
    };
    key_from_contents(&contents, fallback.as_ref().map(|value| value.as_str()))
}

fn key_from_contents(
    contents: &str,
    fallback: Option<&str>,
) -> Result<Option<Zeroizing<String>>, String> {
    if contents.len() > MAX_AUTH_BYTES {
        return Err("OpenCode authentication file is too large.".into());
    }
    if !contents.trim_start().starts_with('{') {
        return Err("OpenCode authentication is invalid. Sign in with OpenCode again.".into());
    }
    // Deserialize only the Go entry; unrelated provider secrets are never retained.
    #[derive(Deserialize)]
    struct Auth {
        #[serde(rename = "opencode-go")]
        go: Option<GoAuth>,
    }
    #[derive(Deserialize)]
    struct GoAuth {
        #[serde(rename = "type")]
        kind: String,
        key: Option<String>,
    }
    let auth: Auth = serde_json::from_str(contents)
        .map_err(|_| "OpenCode authentication is invalid. Sign in with OpenCode again.")?;
    let stored = match auth.go {
        Some(auth) => {
            let key = auth.key.map(Zeroizing::new);
            if auth.kind != "api" || key.as_ref().is_none_or(|key| key.trim().is_empty()) {
                return Err("OpenCode Go needs an API key. Reconnect OpenCode Go.".into());
            }
            key
        }
        None => fallback.map(|key| Zeroizing::new(key.to_string())),
    };
    Ok(stored
        .filter(|key| !key.trim().is_empty())
        .map(|key| Zeroizing::new(key.trim().to_string())))
}

pub(super) fn read(credentials: &[Zeroizing<String>]) -> Result<AccountUsage, String> {
    let client = http::client()?;
    let value = first_accepted(credentials, |credential| {
        let request = client
            .get("https://opencode.ai/zen/go/v1/usage")
            .timeout(Duration::from_secs(5))
            .header("Accept", "application/json");
        let authorization = Zeroizing::new(format!("Bearer {credential}"));
        let request =
            http::secret(request, false, &authorization).map_err(|message| http::RequestError {
                message,
                retryable: false,
                rejected: false,
            })?;
        http::json_result(request, "OpenCode Go")
    })?;
    parse(&value, chrono::Utc::now().timestamp())
}

/// Tries each credential in turn and moves on only when one is refused.
fn first_accepted(
    credentials: &[Zeroizing<String>],
    mut fetch: impl FnMut(&str) -> Result<Value, http::RequestError>,
) -> Result<Value, String> {
    let mut refused = None;
    for credential in credentials {
        match fetch(credential) {
            Ok(value) => return Ok(value),
            Err(error) if error.rejected => refused = Some(error.message),
            Err(error) => return Err(error.message),
        }
    }
    Err(refused.unwrap_or_else(|| "Connect OpenCode Go in Menu Bar settings.".into()))
}

fn parse(value: &Value, now: i64) -> Result<AccountUsage, String> {
    let mut usage = AccountUsage {
        observed_at: now,
        plan: Some("Go".into()),
        source: Some("OpenCode Go".into()),
        ..Default::default()
    };
    for (field, id, label, minutes) in [
        ("rolling", "session", "5-hour", Some(300)),
        ("weekly", "weekly", "Weekly", Some(10080)),
        ("monthly", "monthly", "Monthly", None),
    ] {
        let raw = &value["usage"][field];
        let percent = raw["percent"].as_f64().filter(|n| n.is_finite());
        let resets_at = raw["resetsAt"]
            .as_str()
            .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
            .map(|date| date.timestamp());
        if percent.is_none() || resets_at.is_none() {
            return Err("OpenCode Go returned an unrecognized usage response. Try Refresh.".into());
        }
        usage.windows.push(window(
            id,
            label,
            percent.map(|n| n.clamp(0.0, 100.0)),
            resets_at,
            minutes,
        ));
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_go_key_overrides_environment_without_using_zen_or_other_credentials() {
        let key = key_from_contents(r#"{"opencode-go":{"type":"api","key":" go-key "},"opencode":{"type":"api","key":"zen-key"}}"#, Some("environment-key")).unwrap().unwrap();
        assert_eq!(key.as_str(), "go-key");
        assert!(
            key_from_contents(r#"{"opencode":{"type":"api","key":"zen-key"}}"#, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            key_from_contents("{}", Some(" environment-key "))
                .unwrap()
                .unwrap()
                .as_str(),
            "environment-key"
        );
    }

    #[test]
    fn invalid_auth_does_not_fall_back_to_a_different_account() {
        for text in [
            "broken",
            "[]",
            r#"{"opencode-go":{"type":"oauth","key":"other"}}"#,
            r#"{"opencode-go":{"type":"api","key":""}}"#,
        ] {
            assert!(key_from_contents(text, Some("fallback-secret")).is_err());
        }
        assert!(key_from_contents(&" ".repeat(MAX_AUTH_BYTES + 1), None).is_err());
    }

    #[test]
    fn auth_override_is_authoritative_and_disk_reads_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, "invalid disk content").unwrap();
        let key = key_from_sources(
            Some(Zeroizing::new(
                r#"{"opencode-go":{"type":"api","key":"override"}}"#.into(),
            )),
            Some(path.clone()),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(key.as_str(), "override");
        assert!(key_from_sources(
            None,
            Some(path.clone()),
            Some(Zeroizing::new("fallback".into()))
        )
        .is_err());
        std::fs::write(&path, r#"{"opencode-go":{"type":"api","key":"disk-key"}}"#).unwrap();
        assert_eq!(
            key_from_sources(None, Some(path.clone()), None)
                .unwrap()
                .unwrap()
                .as_str(),
            "disk-key"
        );
        std::fs::write(&path, vec![b' '; MAX_AUTH_BYTES + 1]).unwrap();
        assert!(key_from_sources(None, Some(path.clone()), None).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(key_from_sources(None, Some(path), None).unwrap().is_none());
    }

    fn store(rows: &[(&str, Option<i64>, i64, serde_json::Value)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE credential (id text PRIMARY KEY, integration_id text, label text NOT NULL, value text NOT NULL, connector_id text, method_id text, active integer, time_created integer NOT NULL, time_updated integer NOT NULL);").unwrap();
        for (index, (integration, active, updated, value)) in rows.iter().enumerate() {
            db.execute(
                "INSERT INTO credential VALUES (?1, ?2, 'label', ?3, NULL, NULL, ?4, 0, ?5)",
                rusqlite::params![
                    format!("cred_{index}"),
                    integration,
                    value.to_string(),
                    active,
                    updated
                ],
            )
            .unwrap();
        }
        (dir, path)
    }

    fn account(access: &str, expires: i64, server: &str) -> Value {
        json!({"type": "oauth", "methodID": "device", "refresh": "refresh", "access": access, "expires": expires, "metadata": {"server": server, "orgName": "Default"}})
    }

    fn tokens(found: Option<Vec<Zeroizing<String>>>) -> Vec<String> {
        found
            .unwrap()
            .iter()
            .map(|token| token.to_string())
            .collect()
    }

    #[test]
    fn opencode_2_store_tries_the_go_key_then_the_signed_in_account() {
        let (_dir, path) = store(&[
            (
                "opencode",
                Some(1),
                1,
                account(" access ", 2_000, "https://opencode.ai/console"),
            ),
            (
                "openai",
                None,
                1,
                json!({"type": "key", "key": "openai-key"}),
            ),
            (
                "opencode-go",
                None,
                1,
                json!({"type": "key", "key": "old-go-key"}),
            ),
            (
                "opencode-go",
                None,
                2,
                json!({"type": "key", "key": " go-key "}),
            ),
        ]);
        assert_eq!(
            tokens(stored_credentials(&path, 1_000).unwrap()),
            ["go-key", "access"]
        );
    }

    #[test]
    fn opencode_2_store_skips_expired_or_foreign_sign_ins() {
        let (_dir, path) = store(&[(
            "opencode",
            Some(1),
            1,
            account("access", 1_000, "https://console.example.com"),
        )]);
        assert!(tokens(stored_credentials(&path, 0).unwrap()).is_empty());
        let (_dir, path) = store(&[(
            "opencode",
            Some(1),
            1,
            account("access", 1_000, "https://opencode.ai/console"),
        )]);
        assert!(stored_credentials(&path, 1_000)
            .unwrap_err()
            .contains("expired"));
        let (_dir, path) = store(&[
            (
                "opencode",
                Some(1),
                1,
                account("access", 1_000, "https://opencode.ai/console"),
            ),
            (
                "opencode-go",
                None,
                1,
                json!({"type": "key", "key": "go-key"}),
            ),
        ]);
        assert_eq!(
            tokens(stored_credentials(&path, 1_000).unwrap()),
            ["go-key"]
        );
    }

    #[test]
    fn a_database_without_a_credential_table_is_opencode_1() {
        let dir = tempfile::tempdir().unwrap();
        assert!(stored_credentials(&dir.path().join("opencode.db"), 0)
            .unwrap()
            .is_none());
        let path = dir.path().join("opencode.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE session (id text);")
            .unwrap();
        assert!(stored_credentials(&path, 0).unwrap().is_none());
    }

    fn refused(message: &str, rejected: bool) -> http::RequestError {
        http::RequestError {
            message: message.into(),
            retryable: false,
            rejected,
        }
    }

    #[test]
    fn only_a_refused_credential_moves_on_to_the_next() {
        let credentials = [
            Zeroizing::new("expired".to_string()),
            Zeroizing::new("account".to_string()),
        ];
        let mut tried = Vec::new();
        let value = first_accepted(&credentials, |token| {
            tried.push(token.to_string());
            if token == "expired" {
                Err(refused("expired key", true))
            } else {
                Ok(response())
            }
        })
        .unwrap();
        assert_eq!(tried, ["expired", "account"]);
        assert_eq!(value, response());

        let mut tried = 0;
        let error = first_accepted(&credentials, |_| {
            tried += 1;
            Err(refused("timed out", false))
        })
        .unwrap_err();
        assert_eq!((tried, error.as_str()), (1, "timed out"));

        let error = first_accepted(&credentials, |token| Err(refused(token, true))).unwrap_err();
        assert_eq!(error, "account");
    }

    fn response() -> Value {
        json!({"usage": {
            "rolling": {"percent": 0, "resetsAt": "2026-09-19T17:00:00Z"},
            "weekly": {"percent": 0.5, "resetsAt": "2026-09-25T00:00:00Z"},
            "monthly": {"percent": 110, "resetsAt": "2026-10-01T00:00:00Z"}
        }})
    }

    #[test]
    fn quota_windows_use_reported_percentages_not_fractional_utilization() {
        let usage = parse(&response(), 123).unwrap();
        assert_eq!(usage.plan.as_deref(), Some("Go"));
        assert_eq!(usage.observed_at, 123);
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|window| window.used_percent.value)
                .collect::<Vec<_>>(),
            vec![Some(0.0), Some(0.5), Some(100.0)]
        );
        assert_eq!(usage.windows[0].window_minutes, Some(300));
        assert_eq!(usage.windows[1].window_minutes, Some(10080));
        assert_eq!(usage.windows[2].window_minutes, None);
        assert_eq!(usage.windows[0].resets_at, Some(1789837200));
    }

    #[test]
    fn missing_null_or_malformed_quota_is_not_zero() {
        for field in ["rolling", "weekly", "monthly"] {
            for invalid in [
                Value::Null,
                json!({"percent": 0}),
                json!({"percent": "0", "resetsAt":"2026-09-19T17:00:00Z"}),
                json!({"percent": 0, "resetsAt":"invalid"}),
            ] {
                let mut value = response();
                value["usage"][field] = invalid;
                assert!(parse(&value, 123).is_err());
            }
        }
    }
}
