//! Bounded, read-only account RPCs using the same Codex runtime and credential
//! selection as Bridge sessions. No turn or thread is started, and no token is
//! copied into Bridge or returned to presentation clients.
use super::{resolve_runtime, SpawnedChildGuard};
use crate::{adapters, binary, BridgeError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    process::Stdio,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuota {
    pub account: Option<String>,
    pub plan: Option<String>,
    pub observed_at: i64,
    pub limits: Value,
    /// Independent named pools returned by Codex app-server. Default keeps
    /// snapshots written before app-server exposed these pools readable.
    #[serde(default)]
    pub rate_limits_by_limit_id: BTreeMap<String, Value>,
}

pub fn read() -> Result<AccountQuota, BridgeError> {
    let binary = resolve_runtime().ok_or_else(|| {
        BridgeError::Adapter(
            "Codex is not installed. Install it in Bridge's Harnesses settings.".into(),
        )
    })?;
    let mut command = adapters::supervised_command(&binary, ["app-server", "--listen", "stdio://"]);
    binary::hydrate_command_path(&mut command);
    adapters::configure_process_group(&mut command);
    let mut child = SpawnedChildGuard::new(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let mut stdin = child
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Adapter("Codex account input unavailable".into()))?;
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Adapter("Codex account output unavailable".into()))?;
    let (send, receive) = mpsc::sync_channel(32);
    // Limits the complete transcript of this short-lived probe, including a
    // malformed or unterminated line. The guard kills/reaps on every exit path.
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(1_048_576)).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if value.get("id").is_some() && send.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(12);
    let result = (|| {
        let mut request = |id: u64, method: &str, params: Value| -> Result<Value, BridgeError> {
            writeln!(
                stdin,
                "{}",
                json!({"id": id, "method": method, "params": params})
            )?;
            stdin.flush()?;
            loop {
                let response = receive
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| {
                        BridgeError::Adapter("Codex account refresh timed out. Try again.".into())
                    })?;
                if response.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if response.get("error").is_some() {
                    return Err(BridgeError::Adapter("Codex could not read account usage. Check its sign-in in Harnesses settings.".into()));
                }
                if id == 1 {
                    writeln!(stdin, "{}", json!({"method":"initialized"}))?;
                    stdin.flush()?;
                }
                return response.get("result").cloned().ok_or_else(|| {
                    BridgeError::Adapter("Codex returned an invalid account response".into())
                });
            }
        };
        request(
            1,
            "initialize",
            json!({"clientInfo":{"name":"bridge_usage","title":"Bridge usage","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}}),
        )?;
        let before = request(2, "account/read", json!({"refreshToken": false}))?;
        let limits = request(3, "account/rateLimits/read", Value::Null)?;
        let after = request(4, "account/read", json!({"refreshToken": false}))?;
        normalize(before, after, limits, chrono::Utc::now().timestamp())
    })();
    drop(receive);
    drop(stdin);
    drop(child);
    let _ = reader.join();
    result
}

fn normalize(
    before: Value,
    after: Value,
    limits: Value,
    now: i64,
) -> Result<AccountQuota, BridgeError> {
    if before.get("account") != after.get("account") {
        return Err(BridgeError::Adapter(
            "Codex account changed during refresh. Refresh again.".into(),
        ));
    }
    let account = after
        .get("account")
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            BridgeError::Adapter(
                "Sign in to Codex in Harnesses settings to read account usage.".into(),
            )
        })?;
    // Prefer the named Codex pool; never collapse several independent pools
    // into the largest percentage or invent a token budget from a percentage.
    let pools = limits
        .get("rateLimitsByLimitId")
        .or_else(|| limits.get("rate_limits_by_limit_id"))
        .and_then(Value::as_object);
    let selected = pools
        .and_then(|values| {
            values
                .iter()
                .find(|(id, _)| {
                    bounded_text(id, 128).is_some_and(|id| id.eq_ignore_ascii_case("codex"))
                })
                .map(|(_, value)| value)
        })
        .or_else(|| limits.get("rateLimits"))
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    Ok(AccountQuota {
        account: account
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned),
        plan: account
            .get("planType")
            .or_else(|| selected.get("planType"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        observed_at: now,
        limits: selected,
        rate_limits_by_limit_id: bounded_pools(pools),
    })
}

fn bounded_pools(pools: Option<&serde_json::Map<String, Value>>) -> BTreeMap<String, Value> {
    let mut values: Vec<_> = pools
        .into_iter()
        .flat_map(|values| values.iter())
        .filter(|(_, value)| value.is_object())
        .collect();
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    values
        .into_iter()
        .take(32)
        .filter_map(|(id, value)| {
            let id = bounded_text(id, 128)?;
            let mut value = value.clone();
            if let Some(object) = value.as_object_mut() {
                for key in ["limitName", "limit_name"] {
                    if let Some(label) = object.get(key).and_then(Value::as_str) {
                        match bounded_text(label, 80) {
                            Some(label) => object.insert(key.into(), Value::String(label)),
                            None => object.remove(key),
                        };
                    }
                }
            }
            Some((id, value))
        })
        .collect()
}

fn bounded_text(value: &str, maximum: usize) -> Option<String> {
    let value: String = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_codex_pool_and_preserves_reported_zero() {
        let account = json!({"account":{"email":"person@example.test","planType":"pro"}});
        let value = normalize(
            account.clone(),
            account,
            json!({"rateLimits":{"primary":{"usedPercent":99}},
            "rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":0,"resetsAt":200}}}}),
            100,
        )
        .unwrap();
        assert_eq!(value.limits["primary"]["usedPercent"], 0);
        assert_eq!(
            value.rate_limits_by_limit_id["codex"]["primary"]["usedPercent"],
            0
        );
        assert_eq!(value.account.as_deref(), Some("person@example.test"));
    }
    #[test]
    fn account_switch_drops_inflight_limits() {
        assert!(normalize(
            json!({"account":{"email":"a"}}),
            json!({"account":{"email":"b"}}),
            json!({}),
            100
        )
        .is_err());
    }
    #[test]
    fn named_pools_are_bounded_sanitized_and_match_codex_case_insensitively() {
        let account = json!({"account":{"email":"person@example.test"}});
        let mut pools = serde_json::Map::new();
        pools.insert("CODEX".into(), json!({"primary":{"usedPercent":7}}));
        pools.insert(
            " extra\n".into(),
            json!({"limitName":"  Model\nlimit  ","primary":{"usedPercent":3}}),
        );
        for index in 0..40 {
            pools.insert(
                format!("pool-{index:02}"),
                json!({"primary":{"usedPercent":index}}),
            );
        }
        let value = normalize(
            account.clone(),
            account,
            json!({"rateLimitsByLimitId":pools}),
            100,
        )
        .unwrap();
        assert_eq!(value.limits["primary"]["usedPercent"], 7);
        assert_eq!(value.rate_limits_by_limit_id.len(), 32);
        assert_eq!(
            value.rate_limits_by_limit_id["extra"]["limitName"],
            "Modellimit"
        );
    }
}
