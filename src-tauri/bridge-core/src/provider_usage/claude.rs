use super::*;
use std::path::PathBuf;

fn credentials() -> Result<(String, Option<String>), String> {
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if token.is_empty() || token.len() > 16_384 {
            return Err("Invalid Claude OAuth token".into());
        }
        return Ok((token, None));
    }
    let configured = std::env::var_os("CLAUDE_CONFIG_DIR");
    let path = configured
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude")
        })
        .join(".credentials.json");
    let content = if path.exists() || configured.is_some() {
        super::credentials::read_file(&path)?
    } else {
        String::from_utf8(super::credentials::keychain(
            "Claude Code-credentials",
            None,
        )?)
        .map_err(|_| "Invalid Claude credentials")?
    };
    decode_credentials(&content, chrono::Utc::now().timestamp_millis())
}
fn decode_credentials(content: &str, now_ms: i64) -> Result<(String, Option<String>), String> {
    let data: Value = serde_json::from_str(content).map_err(|_| "Invalid Claude credentials")?;
    let oauth = &data["claudeAiOauth"];
    if oauth["expiresAt"]
        .as_i64()
        .is_some_and(|v| v <= now_ms + 60_000)
    {
        return Err("Claude session expired. Sign in again through Claude Code.".into());
    }
    let token = oauth["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 16_384)
        .ok_or("Sign in through Claude Code to read account limits.")?;
    Ok((token.into(), public_text(&oauth["subscriptionType"])))
}
pub(super) fn read() -> Result<AccountUsage, String> {
    let (token, plan) = credentials()?;
    let client = http::client()?;
    let auth = format!("Bearer {token}");
    let usage = http::json(
        http::secret(
            client
                .get("https://api.anthropic.com/api/oauth/usage")
                .header("anthropic-beta", "oauth-2025-04-20")
                .header("User-Agent", "claude-code/2.1.0")
                .header("Accept", "application/json"),
            false,
            &auth,
        )?,
        "Claude",
    )?;
    let mut parsed = parse(&usage, chrono::Utc::now().timestamp())?;
    parsed.plan = plan;
    // Profile is optional. Never infer identity from a previously cached account.
    if let Ok(profile) = http::json(
        http::secret(
            client.get("https://api.anthropic.com/api/oauth/profile"),
            false,
            &auth,
        )?,
        "Claude",
    ) {
        for key in ["email_address", "emailAddress", "email"] {
            parsed.account =
                public_text(&profile["account"][key]).or_else(|| public_text(&profile[key]));
            if parsed.account.is_some() {
                break;
            }
        }
    }
    Ok(parsed)
}
fn parse(value: &Value, now: i64) -> Result<AccountUsage, String> {
    let mut result = AccountUsage {
        observed_at: now,
        ..Default::default()
    };
    for (key, id, label, minutes) in [
        ("five_hour", "session", "Session", 300),
        ("seven_day", "weekly", "Weekly", 10080),
        ("seven_day_opus", "weekly-opus", "Weekly · Opus", 10080),
        (
            "seven_day_sonnet",
            "weekly-sonnet",
            "Weekly · Sonnet",
            10080,
        ),
        (
            "seven_day_oauth_apps",
            "weekly-oauth",
            "Weekly · OAuth apps",
            10080,
        ),
    ] {
        if value[key].is_object() {
            result.windows.push(window(
                id,
                label,
                number(&value[key]["utilization"]),
                timestamp(&value[key]["resets_at"]),
                Some(minutes),
            ));
        }
    }
    if let Some(limits) = value["limits"].as_array() {
        for (index, limit) in limits.iter().take(32).enumerate() {
            if limit["is_active"].as_bool() == Some(false) {
                continue;
            }
            if limit["group"].as_str() != Some("weekly")
                && !limit["kind"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("weekly"))
            {
                continue;
            }
            if let Some(model) = public_text(&limit["scope"]["model"]["display_name"]) {
                result.windows.push(window(
                    &format!("scoped-{index}"),
                    &format!("Weekly · {model}"),
                    number(&limit["percent"]),
                    timestamp(&limit["resets_at"]),
                    Some(10080),
                ));
            }
        }
    }
    if result.windows.is_empty() {
        return Err(
            "Claude returned no supported usage limits. Local usage is still available.".into(),
        );
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn zero_missing_scoped_and_expiry_remain_distinct() {
        let data = parse(&json!({"five_hour":{"utilization":0,"resets_at":"2026-09-12T00:00:00Z"},"seven_day":{"utilization":null},"seven_day_opus":{"utilization":12}}), 10).unwrap();
        assert_eq!(data.windows[0].used_percent.value, Some(0.0));
        assert_eq!(data.windows[1].used_percent.value, None);
        assert_eq!(data.windows[2].label, "Weekly · Opus");
        assert!(parse(&json!({"error":"bad"}), 10).is_err());
        assert!(decode_credentials(
            r#"{"claudeAiOauth":{"accessToken":"secret","expiresAt":1000}}"#,
            1000
        )
        .is_err());
    }
}
