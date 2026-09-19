use super::*;
use regex::Regex;

const SUBSCRIPTION: &str = "7abeebee372f304e050aaaf92be863f4a86490e382f8c79db68fd94040d691b4";
const BILLING: &str = "c83b78a614689c38ebee981f9b39a8b377716db85c1fd7dbab604adc02d3313d";
pub(super) fn read(workspace_override: Option<&str>) -> Result<AccountUsage, String> {
    // A configured Go key is authoritative: never fall back to a different
    // browser account when that key expires or its request fails.
    if let Some(key) = opencode_go::api_key()? {
        return opencode_go::read(&key);
    }
    let session = credentials::opencode_session().map_err(|_|
        "Connect OpenCode Go in Menu Bar settings, or run opencode auth login and select OpenCode Go. Local usage is still available.".to_string())?;
    let workspace = workspace_override.unwrap_or(&session.workspace);
    if !credentials::valid_workspace(workspace) {
        return Err("Choose a valid OpenCode workspace in Menu Bar settings".into());
    }
    let client = http::client()?;
    let fetch = |id: &str| -> Result<String, String> {
        let args = serde_json::to_string(&[workspace]).map_err(|_| "Invalid OpenCode workspace")?;
        let mut url =
            reqwest::Url::parse("https://opencode.ai/_server").expect("fixed provider URL");
        url.query_pairs_mut()
            .append_pair("id", id)
            .append_pair("args", &args);
        let request = client
            .get(url)
            .header("X-Server-Id", id)
            .header(
                "X-Server-Instance",
                format!("server-fn:{}", uuid::Uuid::new_v4()),
            )
            .header("Origin", "https://opencode.ai")
            .header(
                "Referer",
                format!("https://opencode.ai/workspace/{workspace}/billing"),
            )
            .header(
                "Accept",
                "text/javascript, application/json;q=0.9, */*;q=0.8",
            );
        http::text(http::secret(request, true, &session.cookie)?, "OpenCode")
    };
    let subscription = fetch(SUBSCRIPTION)?;
    let now = chrono::Utc::now().timestamp();
    let mut result = match parse_subscription(&subscription, now)? {
        Some(usage) => usage,
        None => parse_billing(&fetch(BILLING)?, now)?,
    };
    result.account = Some(workspace.to_string());
    Ok(result)
}

// SolidStart responses are data encoded as JavaScript. We only recognize
// bounded fields; never execute the response or follow arbitrary references.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!(
        r#"(?:^|[{{,])\s*(?:"{0}"|{0})\s*:\s*(?:\$R\[\d+\]\s*=\s*)?"#,
        regex::escape(key)
    );
    let matched = Regex::new(&pattern).ok()?.find(text)?;
    Some(text[matched.end()..].trim_start())
}
fn raw_number(text: &str, key: &str) -> Option<f64> {
    let raw = field(text, key)?;
    let matched = Regex::new(r"^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?(?:[,}\s]|$)")
        .ok()?
        .find(raw)?;
    raw[matched.start()..matched.end()]
        .trim_end_matches([',', '}'])
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn raw_string(text: &str, key: &str) -> Option<String> {
    let raw = field(text, key)?;
    let raw = raw
        .strip_prefix("new Date(")
        .map(str::trim_start)
        .unwrap_or(raw);
    serde_json::Deserializer::from_str(raw)
        .into_iter::<String>()
        .next()?
        .ok()
        .filter(|v| !v.is_empty())
}
fn object<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let raw = field(text, key)?;
    if !raw.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in raw.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '{' => {
                depth += 1;
                if depth > 64 {
                    return None;
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[..i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}
fn explicit_null(text: &str) -> bool {
    let text = text.trim().trim_end_matches(';').trim();
    text == "null"
        || Regex::new(r"^\$R\[\d+\]\s*=\s*null$")
            .unwrap()
            .is_match(text)
}
fn parse_subscription(text: &str, now: i64) -> Result<Option<AccountUsage>, String> {
    if explicit_null(text) {
        return Ok(None);
    }
    let mut usage = AccountUsage {
        observed_at: now,
        plan: Some("Zen subscription".into()),
        ..Default::default()
    };
    for (field_name, id, label, minutes) in [
        ("rollingUsage", "session", "Session", 300),
        ("weeklyUsage", "weekly", "Weekly", 10080),
    ] {
        if let Some(raw) = object(text, field_name) {
            let percent = [
                "usagePercent",
                "usedPercent",
                "percentUsed",
                "percent",
                "usage_percent",
                "used_percent",
                "utilization",
                "utilizationPercent",
                "utilization_percent",
                "usage",
            ]
            .iter()
            .find_map(|k| raw_number(raw, k))
            // OpenCode's direct fields accept fractional utilization, unlike
            // Cursor's percent fields. Match its dashboard normalization.
            .map(|v| if v <= 1.0 { v * 100.0 } else { v });
            let relative = [
                "resetInSec",
                "resetInSeconds",
                "resetSeconds",
                "reset_sec",
                "reset_in_sec",
                "resetsInSec",
                "resetsInSeconds",
                "resetIn",
                "resetSec",
            ]
            .iter()
            .find_map(|k| raw_number(raw, k))
            .filter(|v| *v <= 366.0 * 86400.0);
            let absolute = [
                "resetAt",
                "resetsAt",
                "reset_at",
                "resets_at",
                "nextReset",
                "next_reset",
                "renewAt",
                "renew_at",
            ]
            .iter()
            .find_map(|k| {
                raw_string(raw, k)
                    .and_then(|s| timestamp(&Value::String(s)))
                    .or_else(|| raw_number(raw, k).map(|n| n as i64))
            });
            usage.windows.push(window(
                id,
                label,
                percent,
                relative.map(|s| now + s as i64).or(absolute),
                Some(minutes),
            ));
        }
    }
    if usage.windows.is_empty() {
        return Err(
            "OpenCode returned an unrecognized subscription response. Reconnect or try Refresh."
                .into(),
        );
    }
    Ok(Some(usage))
}
fn parse_billing(text: &str, now: i64) -> Result<AccountUsage, String> {
    if raw_string(text, "customerID").is_none() {
        return Err("OpenCode billing account is unavailable".into());
    }
    if field(text, "subscription").is_some_and(|s| !s.starts_with("null")) {
        return Err("OpenCode subscription limits are unavailable".into());
    }
    let spent =
        raw_number(text, "monthlyUsage").ok_or("OpenCode monthly spend is unavailable")? / 100.0; // 1e8 fixed-point USD -> micro-USD.
    let limit = raw_number(text, "monthlyLimit").map(|v| v * 1_000_000.0);
    let balance = raw_number(text, "balance").map(|v| v / 100.0);
    let mut usage = AccountUsage {
        observed_at: now,
        plan: Some("Zen pay as you go".into()),
        metrics: vec![
            amount("monthly-spend", "Monthly account spend", Some(spent)),
            amount("monthly-limit", "Monthly spend limit", limit),
            amount("balance", "Prepaid balance", balance),
        ],
        ..Default::default()
    };
    if let Some(limit) = limit.filter(|v| *v > 0.0) {
        usage.windows.push(window(
            "monthly",
            "Monthly budget",
            Some(spent / limit * 100.0),
            None,
            None,
        ));
    }
    // This is an account balance read now, not a claim that the billing cycle
    // resets on the local calendar's next month boundary.
    Ok(usage)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_json_and_solid_without_executing_javascript() {
        let json = r#"{"rollingUsage":{"usagePercent":0,"resetInSec":120},"weeklyUsage":{"usedPercent":25,"resetInSec":1000}}"#;
        let usage = parse_subscription(json, 100).unwrap().unwrap();
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(usage.windows[0].resets_at, Some(220));
        let solid = r#"$R[0]={rollingUsage:$R[1]={usagePercent:3,resetInSec:20},weeklyUsage:{usagePercent:7,resetInSec:40}};"#;
        assert_eq!(
            parse_subscription(solid, 100).unwrap().unwrap().windows[1]
                .used_percent
                .value,
            Some(7.0)
        );
        assert!(parse_subscription("<html>Sign in</html>", 0).is_err());
        assert!(parse_subscription("null", 0).unwrap().is_none());
        assert!(parse_subscription("arbitrary(null)", 0).is_err());
    }
    #[test]
    fn upstream_billing_fixture_and_fractional_utilization() {
        let text = include_str!("fixtures/opencode-billing.txt");
        let billing = parse_billing(text, 100).unwrap();
        assert_eq!(billing.metrics[0].value.value, Some(15_000_000.0));
        assert_eq!(billing.metrics[2].value.value, Some(12_500_000.0));
        let quota = parse_subscription(
            r#"{"usage":{"rollingUsage":{"usagePercent":0.25,"resetInSec":3600}}}"#,
            100,
        )
        .unwrap()
        .unwrap();
        assert_eq!(quota.windows[0].used_percent.value, Some(25.0));
    }
    #[test]
    fn billing_units_missing_limit_and_unknown_are_not_zero() {
        let row = parse_billing(r#"$R[0]={customerID:"cus_a",monthlyUsage:250000000,balance:$R[3]=400000000,monthlyLimit:20,subscription:null}"#, 100).unwrap();
        assert_eq!(row.metrics[0].value.value, Some(2_500_000.0));
        assert_eq!(row.metrics[1].value.value, Some(20_000_000.0));
        assert_eq!(row.windows[0].used_percent.value, Some(12.5));
        assert_eq!(row.windows[0].resets_at, None);
        assert!(parse_billing(r#"{"monthlyUsage":0}"#, 100).is_err());
        assert!(parse_billing(
            r#"{"customerID":"c","monthlyUsage":0,"subscription":{}}"#,
            100
        )
        .is_err());
        let unlimited =
            parse_billing(r#"{"customerID":"c","monthlyUsage":0,"balance":null}"#, 100).unwrap();
        assert!(unlimited.windows.is_empty());
        assert_eq!(unlimited.metrics[0].value.value, Some(0.0));
        assert_eq!(unlimited.metrics[2].value.value, None);
    }
}
