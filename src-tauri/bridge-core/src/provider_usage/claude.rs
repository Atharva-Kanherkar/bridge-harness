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
fn read_with_credentials(token: String, plan: Option<String>) -> Result<AccountUsage, String> {
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
pub(super) fn read() -> Result<AccountUsage, String> {
    let (token, plan) = credentials()?;
    read_with_credentials(token, plan)
}

pub(super) fn read_interactive(core: &crate::BridgeCore) -> Result<AccountUsage, String> {
    // Explicit credentials identify an account chosen by the caller. Never
    // replace that identity with whichever account the global CLI is using.
    if !cli_fallback_allowed(
        std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some(),
        std::env::var_os("CLAUDE_CONFIG_DIR").is_some(),
    ) {
        return read();
    }
    match credentials() {
        Ok((token, plan)) => read_with_credentials(token, plan),
        Err(direct_error) => super::claude_cli::read(core).map_err(|cli_error| {
            format!("{direct_error} Manual Claude CLI fallback failed: {cli_error}")
        }),
    }
}

fn cli_fallback_allowed(environment_token: bool, configured_directory: bool) -> bool {
    !environment_token && !configured_directory
}
fn parse(value: &Value, now: i64) -> Result<AccountUsage, String> {
    let mut result = AccountUsage {
        observed_at: now,
        ..Default::default()
    };
    for (key, id, label, minutes) in [
        ("five_hour", "session", "5-hour", 300),
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
        let mut seen = std::collections::HashSet::new();
        for limit in limits.iter().take(32) {
            // CodexBar's ClaudeScopedWeeklyLimitMapper (928166f) deliberately
            // keeps is_active:false: enforceable Fable limits report that value.
            if limit["group"].as_str() != Some("weekly")
                || limit["kind"].as_str() != Some("weekly_scoped")
            {
                continue;
            }
            let Some(percent) = number(&limit["percent"]) else {
                continue;
            };
            let Some(model) = public_text(&limit["scope"]["model"]["display_name"]) else {
                continue;
            };
            let model_id = public_text(&limit["scope"]["model"]["id"]);
            let slug = scoped_slug(model_id.as_deref().unwrap_or(&model));
            if slug.is_empty()
                || slug == "all-models"
                || slug.ends_with("-all-models")
                || scoped_slug(&model) == "all-models"
                || !seen.insert(slug.clone())
            {
                continue;
            }
            let label = if model.to_lowercase().ends_with(" only") {
                model
            } else {
                format!("{model} only")
            };
            result.windows.push(window(
                &format!("claude-weekly-scoped-{slug}"),
                &format!("Weekly · {label}"),
                Some(percent),
                timestamp(&limit["resets_at"]),
                Some(10080),
            ));
        }
    }
    if result.windows.is_empty() {
        return Err(
            "Claude returned no supported usage limits. Local usage is still available.".into(),
        );
    }
    Ok(result)
}

fn scoped_slug(value: &str) -> String {
    value
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
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
    #[test]
    fn interactive_fallback_never_substitutes_for_explicit_credentials() {
        assert!(cli_fallback_allowed(false, false));
        assert!(!cli_fallback_allowed(true, false));
        assert!(!cli_fallback_allowed(false, true));
        assert!(!cli_fallback_allowed(true, true));
    }

    #[test]
    fn parses_fable_from_the_generic_scoped_limits_array() {
        let data = parse(
            &json!({
                "five_hour": {"utilization": 1.25, "resets_at": "2026-09-12T00:00:00Z"},
                "seven_day": {"utilization": 20},
                "limits": [
                    {
                        "kind": "weekly_scoped",
                        "group": "weekly",
                        "percent": 37.5,
                        "resets_at": "2026-09-15T00:00:00Z",
                        "scope": {"model": {"id": "claude-fable", "display_name": "Fable"}},
                        "is_active": false
                    },
                    {
                        "kind": "weekly_scoped",
                        "group": "weekly",
                        "percent": 99,
                        "scope": {"model": {"id": "claude-fable", "display_name": "Fable"}},
                        "is_active": true
                    },
                    {
                        "kind": "weekly_scoped",
                        "group": "weekly",
                        "scope": {"model": {"display_name": "Unknown"}},
                        "is_active": true
                    },
                    {"kind":"weekly_scoped", "group":"weekly", "percent":99,
                     "scope":{"model":{"id":"claude-all-models", "display_name":"All models"}}},
                    {"kind":"weekly_total", "group":"weekly", "percent":99,
                     "scope":{"model":{"display_name":"Wrong kind"}}},
                    {"kind":"weekly_scoped", "group":"daily", "percent":99,
                     "scope":{"model":{"display_name":"Wrong group"}}}
                ]
            }),
            10,
        )
        .unwrap();
        let fable = data
            .windows
            .iter()
            .find(|window| window.label == "Weekly · Fable only")
            .unwrap();
        assert_eq!(fable.used_percent.value, Some(37.5));
        assert_eq!(fable.resets_at, Some(1_789_430_400));
        assert_eq!(fable.id, "claude-weekly-scoped-claude-fable");
        assert_eq!(data.windows.len(), 3);
    }

    #[test]
    fn scoped_only_accounts_keep_zero_fable_usage_and_stable_identity() {
        let fable = json!({"kind":"weekly_scoped", "group":"weekly", "percent":0,
            "scope":{"model":{"id":"claude-fable", "display_name":"Fable"}}, "is_active":false});
        let first = parse(
            &json!({"five_hour":null, "seven_day":null, "limits":[fable.clone()]}),
            10,
        )
        .unwrap();
        let reordered = parse(&json!({"limits":[{"kind":"other"}, fable]}), 20).unwrap();
        assert_eq!(first.windows.len(), 1);
        assert_eq!(first.windows[0].label, "Weekly · Fable only");
        assert_eq!(first.windows[0].used_percent.value, Some(0.0));
        assert_eq!(first.windows[0].id, reordered.windows[0].id);
        assert_eq!(first.windows[0].window_minutes, Some(10080));
        assert!(first.windows[0].resets_at.is_none());
    }
}
