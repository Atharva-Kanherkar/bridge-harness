use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{types::ValueRef, Connection, OpenFlags, OptionalExtension};
use std::{path::Path, time::Duration};

fn access_token(path: &Path) -> Result<String, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let db = Connection::open_with_flags(path, flags)
        .or_else(|error| {
            let missing_sidecars = !path.with_file_name("state.vscdb-wal").exists()
                && !path.with_file_name("state.vscdb-shm").exists();
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::CannotOpen)
                && missing_sidecars
            {
                let mut url = reqwest::Url::from_file_path(path)
                    .map_err(|_| rusqlite::Error::InvalidPath(path.to_path_buf()))?;
                url.set_query(Some("immutable=1"));
                Connection::open_with_flags(url.as_str(), flags | OpenFlags::SQLITE_OPEN_URI)
            } else {
                Err(error)
            }
        })
        .map_err(|_| "Sign in to Cursor desktop to read account usage".to_string())?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|_| "Cursor authentication database is busy")?;
    let token = db.query_row("SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken' AND length(value)<=16384", [], |row| {
        let bytes = match row.get_ref(0)? { ValueRef::Text(v) | ValueRef::Blob(v) => v, _ => return Ok(None) };
        Ok(decode_text(bytes))
    }).optional().map_err(|_| "Cursor authentication database could not be read")?.flatten();
    token
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Sign in to Cursor desktop to read account usage".into())
}
fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) || bytes.starts_with(&[0xff, 0xfe]) {
        let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        if bytes.len() % 2 != 0 {
            return None;
        }
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        )
        .ok()
    } else {
        String::from_utf8(bytes.to_vec()).ok()
    }
}
fn session(token: &str, now: i64) -> Result<(String, String), String> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3
        || token.len() > 16_384
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Reconnect Cursor desktop".into());
    }
    let data: Value = URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .ok_or("Reconnect Cursor desktop")?;
    if data["exp"].as_i64().is_none_or(|v| v <= now + 60) {
        return Err("Cursor session expired. Sign in again in Cursor desktop.".into());
    }
    let subject = data["sub"]
        .as_str()
        .ok_or("Cursor account identity is missing")?;
    let id = subject.rsplit('|').next().unwrap_or("");
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Invalid Cursor account identity".into());
    }
    Ok((
        subject.into(),
        format!("WorkosCursorSessionToken={id}%3A%3A{token}"),
    ))
}
pub(super) fn read() -> Result<AccountUsage, String> {
    let home = std::env::var_os("HOME").ok_or("Home directory unavailable")?;
    let path =
        Path::new(&home).join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
    let token = access_token(&path)?;
    let now = chrono::Utc::now().timestamp();
    let (subject, cookie) = session(&token, now)?;
    let client = http::client()?;
    let me = http::json(
        http::secret(client.get("https://cursor.com/api/auth/me"), true, &cookie)?,
        "Cursor",
    )?;
    if let Some(actual) = me["sub"].as_str() {
        if actual.rsplit('|').next().map(str::to_lowercase)
            != subject.rsplit('|').next().map(str::to_lowercase)
        {
            return Err("Cursor account changed. Sign in again in Cursor desktop.".into());
        }
    } else if public_text(&me["email"]).is_none() {
        return Err("Cursor account identity is unavailable".into());
    }
    let usage = http::json(
        http::secret(
            client.get("https://cursor.com/api/usage-summary"),
            true,
            &cookie,
        )?,
        "Cursor",
    )?;
    let mut result = parse(&usage, now)?;
    result.account = public_text(&me["email"]).or_else(|| public_text(&me["sub"]));
    Ok(result)
}
fn ratio(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    used.zip(limit)
        .filter(|(_, l)| *l > 0.0)
        .map(|(u, l)| u / l * 100.0)
}
fn parse(value: &Value, now: i64) -> Result<AccountUsage, String> {
    let personal = &value["individualUsage"];
    let plan = &personal["plan"];
    let overall = &personal["overall"];
    let pool = &value["teamUsage"]["pooled"];
    let cents = |v: &Value| number(v).map(|v| v * 10_000.0);
    let auto = number(&plan["autoPercentUsed"]);
    let third_party = number(&plan["apiPercentUsed"]);
    let lanes = auto
        .zip(third_party)
        .map(|(a, b)| (a + b) / 2.0)
        .or(auto)
        .or(third_party);
    let percent = number(&plan["totalPercentUsed"])
        .or(lanes)
        .or_else(|| ratio(number(&plan["used"]), number(&plan["limit"])))
        .or_else(|| ratio(number(&overall["used"]), number(&overall["limit"])));
    let mut result = AccountUsage {
        observed_at: now,
        plan: public_text(&value["membershipType"]),
        ..Default::default()
    };
    let end = timestamp(&value["billingCycleEnd"]);
    if personal.is_object() {
        if plan.is_object() {
            result
                .windows
                .push(window("total", "Total", percent, end, None));
            if auto.is_some() {
                result
                    .windows
                    .push(window("cursor", "Cursor", auto, end, None));
            }
            if third_party.is_some() {
                result
                    .windows
                    .push(window("third-party", "Third Party", third_party, end, None));
            }
            result.metrics.extend([
                amount("plan-spend", "Plan usage", cents(&plan["used"])),
                amount("plan-limit", "Plan allowance", cents(&plan["limit"])),
            ]);
        } else if overall.is_object() {
            result
                .windows
                .push(window("total", "Total", percent, end, None));
            result.metrics.extend([
                amount("personal-spend", "Personal usage", cents(&overall["used"])),
                amount(
                    "personal-limit",
                    "Personal allowance",
                    cents(&overall["limit"]),
                ),
            ]);
        }
    }
    if pool.is_object() {
        result.windows.push(window(
            "team",
            "Team pool · shared",
            ratio(number(&pool["used"]), number(&pool["limit"])),
            end,
            None,
        ));
    }
    for (scope, data) in [
        ("Personal", &personal["onDemand"]),
        ("Team", &value["teamUsage"]["onDemand"]),
    ] {
        if data.is_object() {
            result.metrics.push(amount(
                &format!("{scope}-on-demand"),
                &format!("{scope} on-demand spend"),
                cents(&data["used"]),
            ));
        }
    }
    if result.windows.is_empty() {
        return Err(
            "Cursor returned no supported account usage. Local usage is still available.".into(),
        );
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn dashboard_percent_units_cents_and_shared_limits() {
        let row = parse(&json!({"individualUsage":{"plan":{"used":100,"limit":2000,"totalPercentUsed":0.36},"onDemand":{"used":0}},"teamUsage":{"pooled":{"used":500,"limit":1000}}}), 10).unwrap();
        assert_eq!(row.windows[0].used_percent.value, Some(0.36));
        assert_eq!(row.metrics[0].value.value, Some(1_000_000.0));
        assert_eq!(row.windows[0].label, "Total");
        assert_eq!(row.windows[1].label, "Team pool · shared");
        assert_eq!(row.windows[1].used_percent.value, Some(50.0));
        assert_eq!(
            parse(&json!({"individualUsage":{"plan":{}}}), 10)
                .unwrap()
                .windows[0]
                .used_percent
                .value,
            None
        );
        assert_eq!(
            parse(
                &json!({"individualUsage":{"plan":{"used":1,"limit":0}}}),
                10
            )
            .unwrap()
            .windows[0]
                .used_percent
                .value,
            None
        );
    }
    #[test]
    fn preserves_cursor_subquota_percent_points_without_scaling() {
        let row = parse(
            &json!({
                "individualUsage":{"plan":{
                    "used":86,"limit":2000,
                    "totalPercentUsed":0.441025641025641,
                    "autoPercentUsed":0.36,
                    "apiPercentUsed":0.7111111111111111
                }}
            }),
            10,
        )
        .unwrap();
        assert_eq!(
            row.windows
                .iter()
                .map(|w| w.label.as_str())
                .collect::<Vec<_>>(),
            ["Total", "Cursor", "Third Party"]
        );
        assert_eq!(row.windows[0].used_percent.value, Some(0.441025641025641));
        assert_eq!(row.windows[1].used_percent.value, Some(0.36));
        assert_eq!(row.windows[2].used_percent.value, Some(0.7111111111111111));
    }
    #[test]
    fn auth_reads_text_and_blob_without_mutating_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE ItemTable(key TEXT, value BLOB);")
            .unwrap();
        let bytes: Vec<u8> = "token".encode_utf16().flat_map(u16::to_le_bytes).collect();
        db.execute(
            "INSERT INTO ItemTable VALUES('cursorAuth/accessToken',?1)",
            [bytes],
        )
        .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert_eq!(access_token(&path).unwrap(), "token");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn rejects_expired_or_injected_sessions() {
        let jwt = |sub: &str, exp: i64| {
            format!(
                "abc.{}.sig",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"sub":sub,"exp":exp})).unwrap())
            )
        };
        assert!(session(&jwt("auth0|user_a", 1000), 1)
            .unwrap()
            .1
            .starts_with("WorkosCursorSessionToken=user_a%3A%3A"));
        assert!(session(&jwt("auth0|a;b", 1000), 1).is_err());
        assert!(session(&jwt("auth0|a", 1), 1).is_err());
    }
}
