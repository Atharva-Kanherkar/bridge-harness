//! Shared provider usage authority. The native menu and
//! desktop consume this contract; neither authenticates, scans, or prices.
use crate::{
    codex_adapter::account::AccountQuota,
    usage_pricing::CostSource,
    usage_summary::{self, UsageBucket, UsageResolution, UsageSummaryRequest},
    BridgeCore, BridgeError,
};
use bridge_protocol::messages::{
    UsageDailyOverview, UsageMetric, UsageMetricSource as Source, UsageMetricStatus as Status,
    UsageModelOverview, UsageOverviewSnapshot, UsagePeriodOverview, UsageQuotaWindow,
};
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex, time::Instant};

#[derive(Default)]
pub struct UsageOverviewService {
    refresh: Mutex<Option<Instant>>,
    providers: [Mutex<Option<Instant>>; 3],
}

#[derive(Default, Serialize, Deserialize)]
struct CachedQuota {
    quota: Option<AccountQuota>,
    error: Option<String>,
}

fn load_cache(db: &Connection) -> Result<CachedQuota, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind='usage_overview' AND id='codex'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    payload
        .map(|p| {
            serde_json::from_str(&p)
                .map_err(|e| BridgeError::Invalid(format!("Invalid usage snapshot: {e}")))
        })
        .transpose()
        .map(|v| v.unwrap_or_default())
}

pub fn snapshot(core: &BridgeCore) -> Result<UsageOverviewSnapshot, BridgeError> {
    let now = Utc::now();
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let tz: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::UTC);
    let today = now.with_timezone(&tz).date_naive();
    let db = core.db.lock().unwrap();
    let cache = load_cache(&db)?;
    let summary = usage_summary::summarize(
        &db,
        &UsageSummaryRequest {
            since_day: (today - Duration::days(29)).to_string(),
            until_day: today.to_string(),
            resolution: UsageResolution::Day,
            time_zone: Some(tz.to_string()),
            workspace_id: None,
            include_imported: true,
            since_time: None,
            until_time: None,
        },
    )?;
    let rows: Vec<_> = summary
        .buckets
        .iter()
        .filter(|b| b.harness == "codex")
        .collect();
    let today_rows: Vec<_> = rows
        .iter()
        .copied()
        .filter(|b| b.day == today.to_string())
        .collect();
    let partial = summary
        .sources
        .iter()
        .any(|s| s.agent == "codex" && s.coverage_state != "complete");
    Ok(UsageOverviewSnapshot {
        schema_version: 1,
        generated_at: now.timestamp(),
        provider: "codex".into(),
        account: cache.quota.as_ref().and_then(|q| q.account.clone()),
        plan: cache.quota.as_ref().and_then(|q| q.plan.clone()),
        observed_at: cache.quota.as_ref().map(|q| q.observed_at),
        windows: windows(cache.quota.as_ref(), cache.error.is_some(), now.timestamp()),
        account_metrics: vec![],
        today: period(&today_rows),
        month: period(&rows),
        daily: daily(&rows),
        coverage: if partial {
            "Recorded on this Mac · history import is incomplete"
        } else {
            "Recorded on this Mac · may include multiple accounts"
        }
        .into(),
        error: cache.error,
    })
}

pub fn refresh(core: &BridgeCore) -> Result<UsageOverviewSnapshot, BridgeError> {
    // One central owner coalesces menu, settings and desktop requests. Never
    // hold the database while waiting for the provider process/network.
    // Concurrent callers join the active refresh instead of returning an old
    // snapshot and prematurely presenting that request as completed.
    let mut last = core
        .usage_overview
        .refresh
        .lock()
        .map_err(|_| BridgeError::Invalid("Usage refresh is unavailable".into()))?;
    if last.is_some_and(|last| last.elapsed().as_secs() < 15) {
        return snapshot(core);
    }
    let prior = load_cache(&core.db.lock().unwrap())?;
    let cache = match crate::codex_adapter::account::read() {
        Ok(quota) => CachedQuota {
            quota: Some(quota),
            error: None,
        },
        // A failed account read may mean credentials changed. Preserve the
        // values as historical, and clear identity so no old quota is labelled
        // as belonging to a new account.
        Err(error) => CachedQuota {
            quota: prior.quota.map(|mut q| {
                q.account = None;
                q.plan = None;
                q
            }),
            error: Some(error.to_string()),
        },
    };
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview','codex',?1,?2,?2)
            ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
            params![serde_json::to_string(&cache).map_err(|e| BridgeError::Invalid(e.to_string()))?, Utc::now().to_rfc3339()])?;
    }
    // Use the existing incremental importer and deduplicating ledger. Source
    // discovery selects Codex only; no second history store or price engine.
    let env = crate::usage_import::SourceEnv::from_process();
    let ids: Vec<_> = crate::usage_import::discover_sources(&env)
        .iter()
        .filter(|s| s.agent == "codex")
        .map(crate::usage_import::source_id_for)
        .collect();
    if !ids.is_empty() {
        let _ = crate::usage_history::scan_history(core, &env, Some(10_000), Some(&ids));
    }
    *last = Some(Instant::now());
    snapshot(core)
}

fn number(value: &Value, camel: &str, snake: &str) -> Option<f64> {
    value
        .get(camel)
        .or_else(|| value.get(snake))
        .and_then(Value::as_f64)
}

fn valid_percent(value: &Value) -> Option<f64> {
    number(value, "usedPercent", "used_percent").filter(|value| value.is_finite() && *value >= 0.0)
}

fn windows(quota: Option<&AccountQuota>, failed: bool, now: i64) -> Vec<UsageQuotaWindow> {
    let primary = quota
        .and_then(|q| q.limits.get("primary"))
        .filter(|v| v.is_object());
    let secondary = quota
        .and_then(|q| q.limits.get("secondary"))
        .filter(|v| v.is_object());
    let minutes =
        |raw: Option<&Value>| raw.and_then(|r| number(r, "windowDurationMins", "window_minutes"));
    // Codex can put its only weekly limit in `primary`, or return the slots
    // reversed. Reported durations define the roles, not their wire positions.
    let (session, weekly) = if (minutes(primary) == Some(10080.0)
        && minutes(secondary) != Some(10080.0))
        || (minutes(secondary) == Some(300.0) && minutes(primary) != Some(300.0))
    {
        (secondary, primary)
    } else {
        (primary, secondary)
    };
    let mut result: Vec<_> = [(session, "session", "5-hour"), (weekly, "weekly", "Weekly")]
        .into_iter()
        .filter_map(|(raw, id, label)| {
            // Do not label a second known weekly window as a session (or vice
            // versa), even if a provider response contains duplicate durations.
            let raw = raw.filter(|r| match minutes(Some(r)) {
                Some(10080.0) => id == "weekly",
                Some(300.0) => id == "session",
                _ => true,
            })?;
            let reset = number(raw, "resetsAt", "resets_at").map(|v| v as i64);
            let duration = number(raw, "windowDurationMins", "window_minutes").map(|v| v as i64);
            let mut used = number(raw, "usedPercent", "used_percent")
                .map(|v| UsageMetric::known(v, Source::Reported))
                .unwrap_or_else(UsageMetric::unavailable);
            // Passing a reset proves the observation expired, not that usage is
            // now zero. A successful re-read is the only way to report fresh zero.
            if used.value.is_some()
                && (failed
                    || quota.is_some_and(|q| now - q.observed_at >= 600 || now < q.observed_at)
                    || reset.is_some_and(|r| r <= now))
            {
                used.status = Status::Stale;
            }
            Some(UsageQuotaWindow {
                id: id.into(),
                label: label.into(),
                used_percent: used,
                resets_at: reset,
                window_minutes: duration,
            })
        })
        .collect();
    if let Some(quota) = quota {
        for (pool_index, (pool_id, pool)) in quota.rate_limits_by_limit_id.iter().enumerate() {
            if pool_id.eq_ignore_ascii_case("codex") || pool == &quota.limits || !pool.is_object() {
                continue;
            }
            let title = pool
                .get("limitName")
                .or_else(|| pool.get("limit_name"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(pool_id);
            for slot in ["primary", "secondary"] {
                let Some(raw) = pool.get(slot).filter(|value| value.is_object()) else {
                    continue;
                };
                if valid_percent(raw).is_none() {
                    continue;
                }
                let label = named_window_label(title, raw);
                result.push(quota_window(
                    raw,
                    format!("codex-{pool_index}-{}-{slot}", bounded_id(pool_id)),
                    label,
                    failed,
                    quota,
                    now,
                ));
            }
        }
    }
    result
}

fn bounded_id(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.chars().take(128) {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !result.is_empty() {
            result.push('-');
            separator = true;
        }
    }
    result.trim_end_matches('-').to_owned()
}

fn named_window_label(title: &str, raw: &Value) -> String {
    let normalized = title.to_ascii_lowercase();
    let duration = number(raw, "windowDurationMins", "window_minutes").map(|value| value as i64);
    let cadence = match duration {
        Some(300) if !normalized.contains("5-hour") && !normalized.contains("5 hour") => " 5-hour",
        Some(10080) if !normalized.contains("weekly") => " Weekly",
        Some(_) | None => "",
    };
    format!("{title}{cadence}")
}

fn quota_window(
    raw: &Value,
    id: String,
    label: String,
    failed: bool,
    quota: &AccountQuota,
    now: i64,
) -> UsageQuotaWindow {
    let reset = number(raw, "resetsAt", "resets_at").map(|value| value as i64);
    let duration = number(raw, "windowDurationMins", "window_minutes").map(|value| value as i64);
    let mut used = valid_percent(raw)
        .map(|value| UsageMetric::known(value, Source::Reported))
        .unwrap_or_else(UsageMetric::unavailable);
    if used.value.is_some()
        && (failed
            || now - quota.observed_at >= 600
            || now < quota.observed_at
            || reset.is_some_and(|value| value <= now))
    {
        used.status = Status::Stale;
    }
    UsageQuotaWindow {
        id,
        label,
        used_percent: used,
        resets_at: reset,
        window_minutes: duration,
    }
}

fn period(rows: &[&UsageBucket]) -> UsagePeriodOverview {
    if rows.is_empty() {
        return UsagePeriodOverview {
            tokens: UsageMetric::unavailable(),
            cost_microusd: UsageMetric::unavailable(),
            models: vec![],
        };
    }
    let mut by_model: BTreeMap<&str, Vec<&UsageBucket>> = BTreeMap::new();
    for row in rows {
        by_model.entry(&row.model).or_default().push(row);
    }
    let mut models: Vec<_> = by_model
        .into_iter()
        .map(|(model, rows)| {
            let input: i64 = rows.iter().map(|r| r.totals.uncached_input_tokens).sum();
            let output: i64 = rows.iter().map(|r| r.totals.output_tokens).sum();
            let cache: i64 = rows
                .iter()
                .map(|r| r.totals.cache_read_tokens + r.totals.cache_write_tokens)
                .sum();
            UsageModelOverview {
                model: model.into(),
                input_tokens: UsageMetric::known(input as f64, Source::Measured),
                output_tokens: UsageMetric::known(output as f64, Source::Measured),
                cache_tokens: UsageMetric::known(cache as f64, Source::Measured),
                total_tokens: UsageMetric::known((input + output + cache) as f64, Source::Measured),
                cost_microusd: cost(&rows),
            }
        })
        .collect();
    models.sort_by(|a, b| {
        b.total_tokens
            .value
            .partial_cmp(&a.total_tokens.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.model.cmp(&b.model))
    });
    UsagePeriodOverview {
        tokens: UsageMetric::known(
            models.iter().filter_map(|m| m.total_tokens.value).sum(),
            Source::Measured,
        ),
        cost_microusd: cost(rows),
        models,
    }
}

fn daily(rows: &[&UsageBucket]) -> Vec<UsageDailyOverview> {
    let mut by_day: BTreeMap<&str, Vec<&UsageBucket>> = BTreeMap::new();
    for row in rows {
        by_day.entry(&row.day).or_default().push(*row);
    }
    by_day
        .into_iter()
        .map(|(day, rows)| UsageDailyOverview {
            day: day.into(),
            usage: period(&rows),
        })
        .collect()
}

fn cost(rows: &[&UsageBucket]) -> UsageMetric {
    if rows
        .iter()
        .any(|r| r.unpriced_records > 0 || r.cost_source == CostSource::Unpriced)
    {
        return UsageMetric::unavailable();
    }
    UsageMetric::known(
        rows.iter().map(|r| r.cost_microusd as f64).sum(),
        if rows
            .iter()
            .all(|r| r.cost_source == CostSource::ProviderReported)
        {
            Source::Reported
        } else {
            Source::Estimated
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reported_durations_identify_weekly_only_and_reversed_windows() {
        let mut quota = AccountQuota {
            account: None,
            plan: None,
            observed_at: 100,
            limits: json!({"primary":{"usedPercent":58,"resetsAt":100000,"windowDurationMins":10080},"secondary":null}),
            rate_limits_by_limit_id: BTreeMap::new(),
        };
        let value = windows(Some(&quota), false, 101);
        assert_eq!(value.len(), 1);
        assert_eq!(value[0].label, "Weekly");
        assert_eq!(value[0].used_percent.value, Some(58.0));
        assert_eq!(value[0].window_minutes, Some(10080));
        quota.limits["secondary"] =
            json!({"usedPercent":0,"resetsAt":200,"windowDurationMins":300});
        let reversed = windows(Some(&quota), false, 101);
        assert_eq!(reversed[0].used_percent.value, Some(0.0));
        assert_eq!(reversed[1].used_percent.value, Some(58.0));
        quota.limits["primary"] = Value::Null;
        assert_eq!(
            windows(Some(&quota), false, 101)[0].used_percent.value,
            Some(0.0)
        );
        quota.limits["secondary"] = Value::Null;
        assert!(windows(Some(&quota), false, 101).is_empty());
    }
    #[test]
    fn named_codex_pools_keep_only_reported_windows_and_titles() {
        let quota = AccountQuota {
            account: None,
            plan: None,
            observed_at: 100,
            limits: json!({}),
            rate_limits_by_limit_id: BTreeMap::from([
                ("codex".into(), json!({"primary":{"usedPercent":99}})),
                (
                    "spark".into(),
                    json!({
                        "limitName":"Codex Spark",
                        "primary":{"usedPercent":4,"windowDurationMins":300},
                        "secondary":{"usedPercent":8,"windowDurationMins":10080}
                    }),
                ),
                ("metadata-only".into(), json!({"limitName":"Metadata only"})),
            ]),
        };
        let value = windows(Some(&quota), false, 101);
        assert_eq!(value.len(), 2);
        assert_eq!(value[0].label, "Codex Spark 5-hour");
        assert_eq!(value[1].label, "Codex Spark Weekly");
        assert_eq!(value[0].used_percent.value, Some(4.0));
        assert_eq!(value[1].used_percent.value, Some(8.0));
    }
    #[test]
    fn named_codex_pools_drop_duplicate_and_malformed_windows_without_cadence_fiction() {
        let selected = json!({"primary":{"usedPercent":2,"windowDurationMins":300}});
        let quota = AccountQuota {
            account: None,
            plan: None,
            observed_at: 100,
            limits: selected.clone(),
            rate_limits_by_limit_id: BTreeMap::from([
                ("CODEX".into(), json!({"primary":{"usedPercent":99}})),
                ("duplicate".into(), selected),
                (
                    "malformed".into(),
                    json!({"primary":{},"secondary":{"usedPercent":-1}}),
                ),
                (
                    "unknown".into(),
                    json!({"limitName":"Model quota","secondary":{"usedPercent":4}}),
                ),
            ]),
        };
        let value = windows(Some(&quota), false, 101);
        assert_eq!(value.len(), 2);
        assert_eq!(value[0].label, "5-hour");
        assert_eq!(value[1].label, "Model quota");
        assert!(value[1].id.len() < 180);
    }
    #[test]
    fn expired_and_failed_reads_keep_stale_values_without_inventing_zero() {
        let q = AccountQuota {
            account: None,
            plan: None,
            observed_at: 100,
            limits: json!({"primary":{"usedPercent":42,"resetsAt":101}, "secondary":{"usedPercent":0,"resetsAt":200}}),
            rate_limits_by_limit_id: BTreeMap::new(),
        };
        let value = windows(Some(&q), false, 102);
        assert_eq!(value[0].used_percent.value, Some(42.0));
        assert_eq!(value[0].used_percent.status, Status::Stale);
        assert_eq!(value[1].used_percent.value, Some(0.0));
        assert_eq!(value[1].used_percent.status, Status::Current);
        assert_eq!(
            windows(Some(&q), true, 102)[1].used_percent.status,
            Status::Stale
        );
        assert!(windows(None, false, 102).is_empty());
    }
    #[test]
    fn pricing_and_token_totals_preserve_missing_data_and_avoid_reasoning_double_count() {
        let mut row = UsageBucket {
            day: "2026-09-10".into(),
            hour_start: None,
            harness: "codex".into(),
            model: "model".into(),
            totals: usage_summary::UsageBucketTotals {
                uncached_input_tokens: 10,
                output_tokens: 20,
                cache_read_tokens: 30,
                cache_write_tokens: 5,
                reasoning_tokens: 15,
            },
            cost_microusd: 0,
            cache_savings_microusd: 0,
            cost_source: CostSource::Unpriced,
            records: 1,
            unpriced_records: 1,
            sessions: 1,
        };
        let usage = period(&[&row]);
        assert_eq!(usage.tokens.value, Some(65.0));
        assert_eq!(usage.cost_microusd.value, None);
        row.cost_source = CostSource::ProviderReported;
        row.unpriced_records = 0;
        assert_eq!(period(&[&row]).cost_microusd.value, Some(0.0));
        row.cost_source = CostSource::ModelPriced;
        assert_eq!(
            period(&[&row]).cost_microusd.source,
            Some(Source::Estimated)
        );
        assert_eq!(period(&[]).tokens.value, None);
    }
    #[test]
    fn daily_series_contains_only_days_with_recorded_rows() {
        let row = |day: &str, tokens: i64| UsageBucket {
            day: day.into(),
            hour_start: None,
            harness: "codex".into(),
            model: "model".into(),
            totals: usage_summary::UsageBucketTotals {
                uncached_input_tokens: tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
            },
            cost_microusd: 0,
            cache_savings_microusd: 0,
            cost_source: CostSource::ProviderReported,
            records: 1,
            unpriced_records: 0,
            sessions: 1,
        };
        let first = row("2026-09-01", 10);
        let third = row("2026-09-03", 30);
        let value = daily(&[&third, &first]);
        assert_eq!(
            value.iter().map(|day| day.day.as_str()).collect::<Vec<_>>(),
            ["2026-09-01", "2026-09-03"]
        );
        assert_eq!(value[0].usage.tokens.value, Some(10.0));
        assert_eq!(value[1].usage.tokens.value, Some(30.0));
    }
}

#[derive(Default, Serialize, Deserialize)]
struct CachedProvider {
    usage: Option<crate::provider_usage::AccountUsage>,
    error: Option<String>,
}
fn load_provider(db: &Connection, provider: &str) -> Result<CachedProvider, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind='usage_overview' AND id=?1",
            [provider],
            |r| r.get(0),
        )
        .optional()?;
    Ok(payload
        .map(|p| {
            serde_json::from_str(&p).unwrap_or_else(|_| CachedProvider {
                usage: None,
                error: Some("Cached account usage is unavailable. Try Refresh.".into()),
            })
        })
        .unwrap_or_default())
}

pub fn provider_snapshots(
    core: &BridgeCore,
) -> Result<bridge_protocol::messages::ProviderUsageOverviews, BridgeError> {
    use bridge_protocol::messages::{MenuBarProvider, ProviderUsageOverviews};
    let now = Utc::now();
    let mut providers = vec![snapshot(core)?];
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let tz: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::UTC);
    let today = now.with_timezone(&tz).date_naive();
    let db = core.db.lock().unwrap();
    let summary = usage_summary::summarize(
        &db,
        &UsageSummaryRequest {
            since_day: (today - Duration::days(29)).to_string(),
            until_day: today.to_string(),
            resolution: UsageResolution::Day,
            time_zone: Some(tz.to_string()),
            workspace_id: None,
            include_imported: true,
            since_time: None,
            until_time: None,
        },
    )?;
    for provider in MenuBarProvider::ALL.into_iter().skip(1) {
        let cache = load_provider(&db, provider.id())?;
        let mut quota = cache.usage.unwrap_or_default();
        expire_provider(&mut quota, cache.error.is_some(), now.timestamp());
        let rows: Vec<_> = summary
            .buckets
            .iter()
            .filter(|b| b.harness == provider.id())
            .collect();
        let todays: Vec<_> = rows
            .iter()
            .copied()
            .filter(|b| b.day == today.to_string())
            .collect();
        let partial = summary
            .sources
            .iter()
            .any(|s| s.agent == provider.id() && s.coverage_state != "complete");
        providers.push(UsageOverviewSnapshot {
            schema_version: 1,
            generated_at: now.timestamp(),
            provider: provider.id().into(),
            account: quota.account,
            plan: quota.plan,
            observed_at: (quota.observed_at > 0).then_some(quota.observed_at),
            windows: quota.windows,
            account_metrics: quota.metrics,
            today: period(&todays),
            month: period(&rows),
            daily: daily(&rows),
            coverage: if provider == MenuBarProvider::Cursor {
                "Recorded by Bridge on this Mac · Cursor history is not imported"
            } else if partial {
                "Recorded on this Mac · history import is incomplete"
            } else {
                "Recorded on this Mac · may include multiple accounts"
            }
            .into(),
            error: cache.error,
        });
    }
    Ok(ProviderUsageOverviews {
        schema_version: 1,
        generated_at: now.timestamp(),
        providers,
    })
}
fn expire_provider(quota: &mut crate::provider_usage::AccountUsage, failed: bool, now: i64) {
    let stale = failed || now < quota.observed_at || now - quota.observed_at >= 600;
    for window in &mut quota.windows {
        if window.used_percent.value.is_some()
            && (stale || window.resets_at.is_some_and(|r| r <= now))
        {
            window.used_percent.status = Status::Stale;
        }
    }
    if stale {
        for amount in &mut quota.metrics {
            if amount.value.value.is_some() {
                amount.value.status = Status::Stale;
            }
        }
    }
}
fn refresh_provider(
    core: &BridgeCore,
    provider: bridge_protocol::messages::MenuBarProvider,
    settings: &bridge_protocol::messages::MenuBarSettings,
    index: usize,
) -> Result<(), BridgeError> {
    let mut last = core.usage_overview.providers[index]
        .lock()
        .map_err(|_| BridgeError::Invalid("Provider refresh unavailable".into()))?;
    if last.is_some_and(|v| v.elapsed().as_secs() < 15) {
        return Ok(());
    }
    let prior = load_provider(&core.db.lock().unwrap(), provider.id())?;
    let cache = match crate::provider_usage::read(provider, settings) {
        Ok(usage) => CachedProvider {
            usage: Some(usage),
            error: None,
        },
        Err(error) => CachedProvider {
            usage: prior.usage.map(|mut q| {
                q.account = None;
                q.plan = None;
                q
            }),
            error: Some(error),
        },
    };
    core.db.lock().unwrap().execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview',?1,?2,?3,?3) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![provider.id(),serde_json::to_string(&cache).map_err(|e| BridgeError::Invalid(e.to_string()))?,Utc::now().to_rfc3339()])?;
    let env = crate::usage_import::SourceEnv::from_process();
    let ids: Vec<_> = crate::usage_import::discover_sources(&env)
        .iter()
        .filter(|s| s.agent == provider.id())
        .map(crate::usage_import::source_id_for)
        .collect();
    if !ids.is_empty() {
        let _ = crate::usage_history::scan_history(core, &env, Some(10_000), Some(&ids));
    }
    *last = Some(Instant::now());
    Ok(())
}
pub fn refresh_providers(
    core: &BridgeCore,
) -> Result<bridge_protocol::messages::ProviderUsageOverviews, BridgeError> {
    use bridge_protocol::messages::MenuBarProvider;
    let settings = crate::menu_bar::load(&core.db.lock().unwrap())?;
    // Independent provider deadlines and caches; one rejected login cannot
    // cancel another provider. No database lock crosses a network operation.
    std::thread::scope(|scope| -> Result<(), BridgeError> {
        let jobs: Vec<_> = MenuBarProvider::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, p)| settings.provider_enabled(*p))
            .map(|(index, p)| {
                let settings = &settings;
                scope.spawn(move || {
                    if p == MenuBarProvider::Codex {
                        refresh(core).map(|_| ())
                    } else {
                        refresh_provider(core, p, settings, index - 1)
                    }
                })
            })
            .collect();
        for job in jobs {
            job.join().map_err(|_| {
                BridgeError::Invalid("Provider collector stopped unexpectedly".into())
            })??;
        }
        Ok(())
    })?;
    provider_snapshots(core)
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    #[test]
    fn failures_and_passed_resets_expire_account_amounts_independently() {
        let mut q = crate::provider_usage::AccountUsage {
            observed_at: 100,
            windows: vec![UsageQuotaWindow {
                id: "monthly".into(),
                label: "Monthly".into(),
                used_percent: UsageMetric::known(0.0, Source::Reported),
                resets_at: Some(110),
                window_minutes: None,
            }],
            metrics: vec![bridge_protocol::messages::UsageAccountMetric {
                id: "balance".into(),
                label: "Balance".into(),
                value: UsageMetric::known(0.0, Source::Reported),
            }],
            ..Default::default()
        };
        expire_provider(&mut q, false, 111);
        assert_eq!(q.windows[0].used_percent.status, Status::Stale);
        assert_eq!(q.metrics[0].value.status, Status::Current);
        expire_provider(&mut q, true, 112);
        assert_eq!(q.metrics[0].value.status, Status::Stale);
        assert_eq!(q.metrics[0].value.value, Some(0.0));
    }
}

/// Account connection changes invalidate both the old identity and refresh gate.
pub fn invalidate_opencode(core: &BridgeCore) -> Result<(), BridgeError> {
    let mut last = core.usage_overview.providers[2]
        .lock()
        .map_err(|_| BridgeError::Invalid("OpenCode refresh unavailable".into()))?;
    core.db.lock().unwrap().execute(
        "DELETE FROM configuration_entries WHERE kind='usage_overview' AND id='opencode'",
        [],
    )?;
    *last = None;
    Ok(())
}
