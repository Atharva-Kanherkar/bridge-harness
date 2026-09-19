use super::*;
use std::path::PathBuf;

fn credentials() -> Result<(String, Option<String>), String> {
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if token.is_empty() || token.len() > 16_384 {
            return Err("Invalid Claude OAuth token".into());
        }
        return Ok((token, None));
    }
    legacy_file_credentials()
}

fn legacy_file_credentials() -> Result<(String, Option<String>), String> {
    let configured = std::env::var_os("CLAUDE_CONFIG_DIR");
    let path = configured
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude")
        })
        .join(".credentials.json");
    let content = super::credentials::read_file(&path)?;
    decode_credentials(&content, chrono::Utc::now().timestamp_millis())
}

fn may_fallback_to_claude_cli(error: &str) -> bool {
    error == "Reconnect Claude to read account usage."
        || error.starts_with("Claude session expired.")
        || error.starts_with("Sign in through Claude Code")
        || error.starts_with("Provider credentials are unavailable")
        || error.starts_with("Provider session is unavailable in Keychain")
        || error.starts_with("Invalid Claude credentials")
        || error.starts_with("Provider session read timed out")
        || error.starts_with("Provider session helper")
        || error.starts_with("Claude Code usage initialization failed.")
        || error.starts_with("Claude Code usage timed out.")
        || error.starts_with("Claude Code could not read usage limits.")
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
    Ok((token.into(), subscription_plan(oauth)))
}

fn subscription_plan(oauth: &Value) -> Option<String> {
    let subscription = public_text(&oauth["subscriptionType"])?;
    // CodexBar's ClaudePlan keeps Max's allowance multiplier separate from
    // utilization. Do not collapse Max 5x and Max 20x into the same plan label.
    let label = match subscription.to_ascii_lowercase().as_str() {
        "max" => "Max",
        "pro" => "Pro",
        "team" => "Team",
        "enterprise" => "Enterprise",
        _ => return Some(subscription),
    };
    if label == "Max" {
        if let Some(tier) = public_text(&oauth["rateLimitTier"]) {
            let words: Vec<_> = tier.split(|c: char| !c.is_ascii_alphanumeric()).collect();
            if let Some(multiplier) = words.windows(2).find_map(|pair| {
                (pair[0].eq_ignore_ascii_case("max")
                    && pair[1].strip_suffix('x').is_some_and(|value| {
                        !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit())
                    }))
                .then_some(pair[1])
            }) {
                return Some(format!("Max ({multiplier})"));
            }
        }
    }
    Some(label.into())
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
pub(super) fn read(core: &crate::BridgeCore) -> Result<AccountUsage, String> {
    read_with_sdk_or_explicit(
        explicit_credentials(),
        || super::claude_sdk::read(core),
        read_explicit_credentials,
    )
}

fn explicit_credentials() -> bool {
    std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some()
        || std::env::var_os("CLAUDE_CONFIG_DIR").is_some()
}

fn read_with_sdk_or_explicit<S, D>(
    explicit: bool,
    mut sdk: S,
    mut direct: D,
) -> Result<AccountUsage, String>
where
    S: FnMut() -> Result<AccountUsage, String>,
    D: FnMut() -> Result<AccountUsage, String>,
{
    if explicit {
        return direct();
    }
    // Claude owns default-profile credentials and renewal. Never inspect its
    // Keychain as a compatibility fallback: background collection must not
    // prompt, and an old credential must not substitute for the active login.
    sdk()
}

fn read_explicit_credentials() -> Result<AccountUsage, String> {
    let (token, plan) = credentials()?;
    read_with_credentials(token, plan)
}

pub(super) fn read_interactive(core: &crate::BridgeCore) -> Result<AccountUsage, String> {
    // Explicit credentials identify an account chosen by the caller. Never
    // replace that identity with whichever account the global CLI is using.
    if explicit_credentials() {
        return read_explicit_credentials();
    }
    read_interactive_with_fallback(
        || super::claude_sdk::read(core),
        || super::claude_cli::read(core),
    )
}

fn read_interactive_with_fallback<D, C>(mut direct: D, mut cli: C) -> Result<AccountUsage, String>
where
    D: FnMut() -> Result<AccountUsage, String>,
    C: FnMut() -> Result<AccountUsage, String>,
{
    match direct() {
        Ok(usage) => Ok(usage),
        Err(direct_error) if may_fallback_to_claude_cli(&direct_error) => {
            cli().map_err(|cli_error| {
                format!("{direct_error} Manual Claude CLI fallback failed: {cli_error}")
            })
        }
        Err(error) => Err(error),
    }
}

pub(super) fn parse(value: &Value, now: i64) -> Result<AccountUsage, String> {
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
    if let Some(scoped) = value["model_scoped"].as_array() {
        let mut seen = std::collections::HashSet::new();
        for limit in scoped.iter().take(32) {
            let Some(model) = public_text(&limit["display_name"]) else { continue };
            let slug = scoped_slug(&model);
            if slug.is_empty() || !seen.insert(slug.clone()) { continue; }
            let label = if model.to_lowercase().ends_with(" only") { model } else { format!("{model} only") };
            result.windows.push(window(
                &format!("claude-weekly-scoped-{slug}"), &format!("Weekly · {label}"),
                number(&limit["utilization"]), timestamp(&limit["resets_at"]), Some(10080),
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
    use std::cell::Cell;

    fn usage() -> AccountUsage {
        AccountUsage {
            observed_at: 1,
            ..Default::default()
        }
    }
    #[test]
    fn automatic_default_refresh_uses_only_claudes_sdk() {
        assert!(read_with_sdk_or_explicit(
            false,
            || Ok(usage()),
            || panic!("default profiles must not read another application's credentials"),
        )
        .is_ok());
        assert_eq!(
            read_with_sdk_or_explicit(
                false,
                || Err("Claude usage SDK unavailable.".into()),
                || panic!("an unavailable SDK must not fall back to Keychain"),
            )
            .unwrap_err(),
            "Claude usage SDK unavailable."
        );
        assert!(read_with_sdk_or_explicit(
            true,
            || panic!("explicit profiles use their selected credentials"),
            || Ok(usage()),
        )
        .is_ok());
    }
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
    fn credential_plan_preserves_max_allowance_without_guessing() {
        for (tier, expected) in [
            ("default_claude_max_5x", "Max (5x)"),
            ("default_claude_max_20x", "Max (20x)"),
            ("default_claude_max_unknown", "Max"),
            ("default_claude_max_x", "Max"),
        ] {
            let (_, plan) = decode_credentials(
                &json!({"claudeAiOauth": {"accessToken": "test", "subscriptionType": "max",
                    "rateLimitTier": tier}}).to_string(), 1,
            ).unwrap();
            assert_eq!(plan.as_deref(), Some(expected));
        }
        assert_eq!(subscription_plan(&json!({"subscriptionType":"pro"})).as_deref(), Some("Pro"));
        assert_eq!(subscription_plan(&json!({"rateLimitTier":"default_claude_max_5x"})), None);
    }

    #[test]
    fn rate_limit_and_transport_failures_do_not_launch_the_cli() {
        for error in [
            "Claude is rate limited. Wait a few minutes before refreshing.",
            "Claude usage request failed or timed out. Try Refresh.",
        ] {
            let cli_calls = Cell::new(0);
            let result = read_interactive_with_fallback(
                || Err(error.into()),
                || {
                    cli_calls.set(cli_calls.get() + 1);
                    Ok(usage())
                },
            );
            assert_eq!(result.unwrap_err(), error);
            assert_eq!(cli_calls.get(), 0);
        }
    }

    #[test]
    fn manual_sdk_recovery_stays_on_claude_but_never_replaces_a_non_subscription_login() {
        for error in ["Claude Code usage initialization failed.", "Claude Code usage timed out. Try Refresh.",
            "Claude Code could not read usage limits. Try Refresh or check its sign-in."] {
            assert!(read_interactive_with_fallback(|| Err(error.into()), || Ok(usage())).is_ok());
        }
        assert!(!may_fallback_to_claude_cli("Claude Code is not reporting subscription limits for this sign-in. Check your Claude Code account."));
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
