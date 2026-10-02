//! OpenCode Go's account usage API; no browser cookies or inference calls.
use super::*;
use std::{io::Read, path::PathBuf, time::Duration};
use zeroize::Zeroizing;

const MAX_AUTH_BYTES: usize = 1_048_576;

pub(super) fn api_key() -> Result<Option<Zeroizing<String>>, String> {
    let content = match std::env::var("OPENCODE_AUTH_CONTENT") {
        Ok(value) if !value.is_empty() => Some(Zeroizing::new(value)),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("OPENCODE_AUTH_CONTENT is not valid text.".into())
        }
        _ => None,
    };
    let path = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|base| base.join("opencode/auth.json"));
    let fallback = std::env::var("OPENCODE_API_KEY").ok().map(Zeroizing::new);
    key_from_sources(content, path, fallback)
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

pub(super) fn read(key: &str) -> Result<AccountUsage, String> {
    let request = http::client()?
        .get("https://opencode.ai/zen/go/v1/usage")
        .timeout(Duration::from_secs(5))
        .header("Accept", "application/json");
    let authorization = Zeroizing::new(format!("Bearer {key}"));
    let value = http::json(http::secret(request, false, &authorization)?, "OpenCode Go")?;
    parse(&value, chrono::Utc::now().timestamp())
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
