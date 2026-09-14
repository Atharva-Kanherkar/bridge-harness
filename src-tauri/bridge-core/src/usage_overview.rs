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
use chrono::{Duration, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
pub struct UsageOverviewService {
    refresh: Mutex<Option<Instant>>,
    providers: [Mutex<Option<Instant>>; 3],
    history: Mutex<Option<CachedSummary>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryKey {
    total_changes: u64,
    data_version: i64,
    today: NaiveDate,
    time_zone: String,
}

struct CachedSummary {
    key: SummaryKey,
    summary: Arc<usage_summary::UsageSummary>,
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

fn summary_key(
    db: &Connection,
    today: NaiveDate,
    time_zone: &str,
) -> Result<SummaryKey, BridgeError> {
    Ok(SummaryKey {
        total_changes: db.total_changes(),
        data_version: db.query_row("PRAGMA data_version", [], |row| row.get(0))?,
        today,
        time_zone: time_zone.to_owned(),
    })
}

fn menu_summary(
    service: &UsageOverviewService,
    db: &Connection,
    today: NaiveDate,
    time_zone: &str,
) -> Result<Arc<usage_summary::UsageSummary>, BridgeError> {
    let before = summary_key(db, today, time_zone)?;
    if let Some(cached) = service.history.lock().unwrap().as_ref() {
        if cached.key == before {
            return Ok(Arc::clone(&cached.summary));
        }
    }
    let summary = Arc::new(usage_summary::summarize(
        db,
        &UsageSummaryRequest {
            since_day: (today - Duration::days(29)).to_string(),
            until_day: today.to_string(),
            resolution: UsageResolution::Day,
            time_zone: Some(time_zone.to_owned()),
            workspace_id: None,
            include_imported: true,
            include_dashboard: false,
            since_time: None,
            until_time: None,
        },
    )?);
    // Another SQLite connection can commit while the read-only aggregation is
    // running. Return that valid result, but do not let it become a cache hit:
    // the next caller must observe the newer database version.
    let after = summary_key(db, today, time_zone)?;
    if before == after {
        *service.history.lock().unwrap() = Some(CachedSummary {
            key: after,
            summary: Arc::clone(&summary),
        });
    }
    Ok(summary)
}

fn codex_snapshot(
    cache: CachedQuota,
    summary: &usage_summary::UsageSummary,
    now: chrono::DateTime<Utc>,
    today: NaiveDate,
) -> UsageOverviewSnapshot {
    let today_string = today.to_string();
    let rows: Vec<_> = summary
        .buckets
        .iter()
        .filter(|b| b.harness == "codex")
        .collect();
    let today_rows: Vec<_> = rows
        .iter()
        .copied()
        .filter(|b| b.day == today_string)
        .collect();
    let partial = summary
        .sources
        .iter()
        .any(|s| s.agent == "codex" && s.coverage_state != "complete");
    UsageOverviewSnapshot {
        schema_version: 1,
        generated_at: now.timestamp(),
        provider: "codex".into(),
        account: cache.quota.as_ref().and_then(|q| q.account.clone()),
        plan: cache.quota.as_ref().and_then(|q| q.plan.clone()),
        quota_source: None,
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
    }
}

pub fn snapshot(core: &BridgeCore) -> Result<UsageOverviewSnapshot, BridgeError> {
    let now = Utc::now();
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let tz: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::UTC);
    let today = now.with_timezone(&tz).date_naive();
    let db = core.db.lock().unwrap();
    let cache = load_cache(&db)?;
    let summary = menu_summary(&core.usage_overview, &db, today, &tz.to_string())?;
    Ok(codex_snapshot(cache, &summary, now, today))
}

pub fn refresh(core: &BridgeCore) -> Result<UsageOverviewSnapshot, BridgeError> {
    refresh_codex(core)?;
    snapshot(core)
}

fn refresh_codex(core: &BridgeCore) -> Result<(), BridgeError> {
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
        return Ok(());
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
    Ok(())
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

fn confirmed_empty_account_period() -> UsagePeriodOverview {
    UsagePeriodOverview {
        tokens: UsageMetric::known(0.0, Source::Reported),
        cost_microusd: UsageMetric::known(0.0, Source::Reported),
        models: vec![],
    }
}

fn unavailable_account_period() -> UsagePeriodOverview {
    UsagePeriodOverview {
        tokens: UsageMetric::unavailable(),
        cost_microusd: UsageMetric::unavailable(),
        models: vec![],
    }
}

fn stale_period(period: &mut UsagePeriodOverview) {
    for metric in [&mut period.tokens, &mut period.cost_microusd] {
        if metric.value.is_some() {
            metric.status = Status::Stale;
        }
    }
    for model in &mut period.models {
        for metric in [
            &mut model.input_tokens,
            &mut model.output_tokens,
            &mut model.cache_tokens,
            &mut model.total_tokens,
            &mut model.cost_microusd,
        ] {
            if metric.value.is_some() {
                metric.status = Status::Stale;
            }
        }
    }
}

fn project_account_history(
    mut history: crate::provider_usage::AccountHistory,
    today: chrono::NaiveDate,
    now: i64,
    refresh_error: Option<&str>,
) -> crate::provider_usage::AccountHistory {
    let crossed_day = history.through_day != today.to_string();
    history.today = if crossed_day {
        unavailable_account_period()
    } else {
        history
            .daily
            .iter()
            .find(|day| day.day == today.to_string())
            .map(|day| day.usage.clone())
            .unwrap_or_else(confirmed_empty_account_period)
    };
    if refresh_error.is_some() || crossed_day || now < history.observed_at || now - history.observed_at >= 600 {
        stale_period(&mut history.today);
        stale_period(&mut history.month);
        for day in &mut history.daily {
            stale_period(&mut day.usage);
        }
        history
            .coverage
            .push_str(" · stale; refresh for current account history");
    }
    if let Some(error) = refresh_error {
        history.coverage.push_str(&format!(" · {error}"));
    }
    history
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
            sessions: Some(1),
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
            sessions: Some(1),
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
pub(crate) struct CachedProvider {
    pub(crate) usage: Option<crate::provider_usage::AccountUsage>,
    pub(crate) error: Option<String>,
}
pub(crate) fn load_provider(db: &Connection, provider: &str) -> Result<CachedProvider, BridgeError> {
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
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let tz: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::UTC);
    let today = now.with_timezone(&tz).date_naive();
    let db = core.db.lock().unwrap();
    let summary = menu_summary(&core.usage_overview, &db, today, &tz.to_string())?;
    let mut providers = vec![codex_snapshot(load_cache(&db)?, &summary, now, today)];
    for provider in MenuBarProvider::ALL.into_iter().skip(1) {
        let cache = load_provider(&db, provider.id())?;
        let mut quota = cache.usage.unwrap_or_default();
        let history_error = quota.history_error.clone();
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
        let mut account_history = (provider == MenuBarProvider::Cursor)
            .then(|| quota.history.take())
            .flatten();
        let (today_usage, month_usage, daily_usage, coverage) = if let Some(history) =
            account_history.take()
        {
            let history = project_account_history(history, today, now.timestamp(), history_error.as_deref());
            (
                history.today,
                history.month,
                history.daily,
                history.coverage,
            )
        } else {
            (
                period(&todays),
                period(&rows),
                daily(&rows),
                if provider == MenuBarProvider::Cursor {
                    format!("Cursor dashboard history unavailable{} · local stores contain no token counts", history_error.as_deref().map(|e| format!(": {e}")).unwrap_or_default())
                } else if partial {
                    "Recorded on this Mac · history import is incomplete".into()
                } else {
                    "Recorded on this Mac · may include multiple accounts".into()
                },
            )
        };
        providers.push(UsageOverviewSnapshot {
            schema_version: 1,
            generated_at: now.timestamp(),
            provider: provider.id().into(),
            account: quota.account,
            plan: quota.plan,
            quota_source: quota.source,
            observed_at: (quota.observed_at > 0).then_some(quota.observed_at),
            windows: quota.windows,
            account_metrics: quota.metrics,
            today: today_usage,
            month: month_usage,
            daily: daily_usage,
            coverage,
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

fn retain_cursor_history(
    usage: &mut crate::provider_usage::AccountUsage,
    prior: Option<&crate::provider_usage::AccountUsage>,
) {
    let retryable = usage.history_error.as_deref().is_some_and(|error| {
        error == "Cursor response could not be read"
            || error == "Cursor history request timed out"
            || error == "Cursor history pagination was inconsistent"
            || error == "Cursor history pagination was incomplete"
            || error.starts_with("Cursor usage request failed or timed out.")
            || error.starts_with("Cursor is rate limited.")
            || error.strip_prefix("Cursor usage is unavailable (HTTP ")
                .and_then(|code| code.strip_suffix(")."))
                .and_then(|code| code.parse::<u16>().ok())
                .is_some_and(|code| code == 408 || (500..600).contains(&code))
    });
    if usage.history.is_some() || !retryable {
        return;
    }
    // An email label is insufficient: only a freshly verified account scope
    // can authorize reuse. Keep observation time and the refresh error so the
    // projection marks these amounts stale even if they were fetched recently.
    if let Some(history) = prior.and_then(|prior| prior.history.as_ref()) {
        if usage.account_scope.as_deref() == Some(history.account_scope.as_str()) {
            usage.history = Some(history.clone());
        }
    }
}

fn cache_provider_result(
    provider: bridge_protocol::messages::MenuBarProvider,
    prior: CachedProvider,
    result: Result<crate::provider_usage::AccountUsage, crate::provider_usage::AccountReadError>,
) -> CachedProvider {
    match result {
        Ok(mut usage) => {
            if provider == bridge_protocol::messages::MenuBarProvider::Cursor {
                retain_cursor_history(&mut usage, prior.usage.as_ref());
            }
            CachedProvider {
                usage: Some(usage),
                error: None,
            }
        }
        Err(error) => CachedProvider {
            usage: prior.usage.map(|mut q| {
                let retain = provider == bridge_protocol::messages::MenuBarProvider::Cursor
                    && error.retry_account_scope.as_deref().is_some_and(|scope| {
                        !scope.is_empty()
                            && q.account_scope.as_deref() == Some(scope)
                            && q.history.as_ref().is_none_or(|history| history.account_scope == scope)
                    });
                if retain {
                    // Keep the original timestamps: a failed refresh cannot
                    // turn yesterday's reading into a fresh one in either UI.
                    q.history_error = Some(error.message.clone());
                } else {
                    q.account = None;
                    q.plan = None;
                    q.account_scope = None;
                    q.history = None;
                }
                q
            }),
            error: Some(error.message),
        },
    }
}

fn refresh_provider(
    core: &BridgeCore,
    provider: bridge_protocol::messages::MenuBarProvider,
    settings: &bridge_protocol::messages::MenuBarSettings,
    index: usize,
    interactive: bool,
) -> Result<(), BridgeError> {
    let mut last = core.usage_overview.providers[index]
        .lock()
        .map_err(|_| BridgeError::Invalid("Provider refresh unavailable".into()))?;
    if !interactive && last.is_some_and(|v| v.elapsed().as_secs() < 15) {
        return Ok(());
    }
    let prior = load_provider(&core.db.lock().unwrap(), provider.id())?;
    let result = if interactive {
        crate::provider_usage::read_interactive(core, provider, settings)
    } else {
        crate::provider_usage::read(provider, settings)
    };
    let cache = cache_provider_result(provider, prior, result);
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
/// History uses the same Cursor collector and refresh gate as the Menu Bar.
/// The provider preference is authoritative; listing sources never authenticates.
pub(crate) fn refresh_cursor_history(core: &BridgeCore) -> Result<(), BridgeError> {
    let settings = crate::menu_bar::load(&core.db.lock().unwrap())?;
    let provider = bridge_protocol::messages::MenuBarProvider::Cursor;
    if settings.provider_enabled(provider) {
        refresh_provider(core, provider, &settings, 1, false)?;
    }
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
                        refresh_codex(core)
                    } else {
                        refresh_provider(core, p, settings, index - 1, false)
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

pub fn refresh_providers_interactive(
    core: &BridgeCore,
) -> Result<bridge_protocol::messages::ProviderUsageOverviews, BridgeError> {
    use bridge_protocol::messages::MenuBarProvider;
    let settings = crate::menu_bar::load(&core.db.lock().unwrap())?;
    std::thread::scope(|scope| -> Result<(), BridgeError> {
        let jobs: Vec<_> = MenuBarProvider::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, provider)| settings.provider_enabled(*provider))
            .map(|(index, provider)| {
                let settings = &settings;
                scope.spawn(move || {
                    if provider == MenuBarProvider::Codex {
                        refresh_codex(core)
                    } else {
                        refresh_provider(core, provider, settings, index - 1, true)
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

    fn test_day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 12).unwrap()
    }

    fn seed_live_usage(db: &Connection) {
        db.execute(
            "INSERT INTO usage_ledger(workspace_id,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,harness,model,source,created_at)
             VALUES('w',1000,0,0,100,'codex','cache-test-model','provider.codex','2026-09-12T12:00:00Z')",
            [],
        )
        .unwrap();
    }

    fn mark_cached(summary: &mut Option<CachedSummary>) {
        Arc::make_mut(&mut summary.as_mut().unwrap().summary).scan_duration_ms = -123;
    }

    #[test]
    fn menu_summary_reuses_one_entry_until_owned_usage_or_pricing_changes() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        let service = UsageOverviewService::default();
        seed_live_usage(&db);

        let first = menu_summary(&service, &db, test_day(), "UTC").unwrap();
        assert_eq!(first.buckets[0].cost_microusd, 0);
        assert_eq!(first.buckets[0].unpriced_records, 1);
        mark_cached(&mut service.history.lock().unwrap());
        assert_eq!(
            menu_summary(&service, &db, test_day(), "UTC")
                .unwrap()
                .scan_duration_ms,
            -123,
            "an unchanged menu install must reuse its aggregate"
        );

        db.execute(
            "INSERT INTO usage_price_overrides(model,input_microusd_per_mtok,output_microusd_per_mtok,cache_read_microusd_per_mtok,cache_write_microusd_per_mtok,updated_at)
             VALUES('cache-test-model',1000000,2000000,100000,1000000,'now')",
            [],
        )
        .unwrap();
        let repriced = menu_summary(&service, &db, test_day(), "UTC").unwrap();
        assert_ne!(repriced.scan_duration_ms, -123);
        assert_eq!(repriced.buckets[0].cost_microusd, 1200);
        assert_eq!(repriced.buckets[0].unpriced_records, 0);

        db.execute("DELETE FROM usage_ledger", []).unwrap();
        assert!(menu_summary(&service, &db, test_day(), "UTC")
            .unwrap()
            .buckets
            .is_empty());
    }

    #[test]
    fn menu_summary_invalidates_for_other_connections_and_calendar_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage-cache.db");
        let db = crate::store::open(&path).unwrap();
        let service = UsageOverviewService::default();
        seed_live_usage(&db);
        menu_summary(&service, &db, test_day(), "UTC").unwrap();
        mark_cached(&mut service.history.lock().unwrap());

        let other = Connection::open(&path).unwrap();
        other
            .execute(
                "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
                 VALUES('cache-test','external','{}','now','now')",
                [],
            )
            .unwrap();
        assert_ne!(
            menu_summary(&service, &db, test_day(), "UTC")
                .unwrap()
                .scan_duration_ms,
            -123,
            "PRAGMA data_version must catch commits made by another connection"
        );

        mark_cached(&mut service.history.lock().unwrap());
        let tomorrow = test_day().succ_opt().unwrap();
        assert_ne!(
            menu_summary(&service, &db, tomorrow, "UTC")
                .unwrap()
                .scan_duration_ms,
            -123
        );
        mark_cached(&mut service.history.lock().unwrap());
        assert_ne!(
            menu_summary(&service, &db, tomorrow, "America/Los_Angeles")
                .unwrap()
                .scan_duration_ms,
            -123
        );
    }

    #[test]
    fn cached_history_does_not_freeze_snapshot_time_or_quota_expiry() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        let service = UsageOverviewService::default();
        seed_live_usage(&db);
        let summary = menu_summary(&service, &db, test_day(), "UTC").unwrap();
        let reused = menu_summary(&service, &db, test_day(), "UTC").unwrap();
        assert!(Arc::ptr_eq(&summary, &reused));

        let quota = AccountQuota {
            account: Some("fixture@example.com".into()),
            plan: Some("fixture".into()),
            observed_at: 100,
            limits: serde_json::json!({
                "primary": {
                    "usedPercent": 42,
                    "resetsAt": 150,
                    "windowDurationMins": 300
                }
            }),
            rate_limits_by_limit_id: BTreeMap::new(),
        };
        let cache = || CachedQuota {
            quota: Some(quota.clone()),
            error: None,
        };
        let before = codex_snapshot(
            cache(),
            &reused,
            chrono::DateTime::from_timestamp(120, 0).unwrap(),
            test_day(),
        );
        let after = codex_snapshot(
            cache(),
            &reused,
            chrono::DateTime::from_timestamp(151, 0).unwrap(),
            test_day(),
        );

        assert_eq!(before.generated_at, 120);
        assert_eq!(after.generated_at, 151);
        assert_eq!(before.windows[0].used_percent.status, Status::Current);
        assert_eq!(after.windows[0].used_percent.status, Status::Stale);
        assert_eq!(after.windows[0].used_percent.value, Some(42.0));
        assert_eq!(before.daily, after.daily);
    }

    #[test]
    fn cursor_history_does_not_roll_yesterdays_snapshot_into_today() {
        let yesterday = chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        let today = yesterday.succ_opt().unwrap();
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-09-10T23:59:00Z")
            .unwrap()
            .timestamp();
        let history = crate::provider_usage::AccountHistory {
            account_scope: "scope".into(),
            observed_at,
            through_day: yesterday.to_string(),
            today: confirmed_empty_account_period(),
            month: confirmed_empty_account_period(),
            daily: vec![],
            coverage: "Cursor dashboard account history".into(),
            breakdown: None,
        };
        let projected = project_account_history(history, today, observed_at + 120, None);
        assert_eq!(projected.today.tokens.value, None);
        assert_eq!(projected.today.tokens.status, Status::Unavailable);
        assert_eq!(projected.month.tokens.status, Status::Stale);
    }

    #[test]
    fn cursor_history_survives_reload_and_a_same_account_transient_failure() {
        use crate::provider_usage::{AccountHistory, AccountUsage};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.db");
        let day = test_day();
        let observed_at = day.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let prior = CachedProvider {
            usage: Some(AccountUsage {
                history: Some(AccountHistory {
                    account_scope: "verified-account-a".into(),
                    observed_at,
                    through_day: day.to_string(),
                    today: confirmed_empty_account_period(),
                    month: confirmed_empty_account_period(),
                    daily: vec![],
                    coverage: "Cursor dashboard account history".into(),
                    breakdown: None,
                }),
                ..Default::default()
            }),
            error: None,
        };
        {
            let db = crate::store::open(&path).unwrap();
            db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview','cursor',?1,'now','now')",
                [serde_json::to_string(&prior).unwrap()]).unwrap();
        }
        let db = crate::store::open(&path).unwrap();
        let prior = load_provider(&db, "cursor").unwrap();
        let mut current = AccountUsage {
            account_scope: Some("verified-account-a".into()),
            observed_at: observed_at + 60,
            history_error: Some("Cursor response could not be read".into()),
            ..Default::default()
        };
        retain_cursor_history(&mut current, prior.usage.as_ref());
        assert_eq!(current.observed_at, observed_at + 60);
        assert_eq!(current.history.as_ref().unwrap().observed_at, observed_at);
        let projected = project_account_history(current.history.unwrap(), day, observed_at + 60, current.history_error.as_deref());
        assert_eq!(projected.month.cost_microusd.value, Some(0.0));
        assert_eq!(projected.month.cost_microusd.status, Status::Stale);
        assert_eq!(projected.today.tokens.status, Status::Stale);
        assert!(projected.coverage.contains("response could not be read"));
    }

    #[test]
    fn cursor_history_retention_requires_verified_identity_and_a_transient_error() {
        use crate::provider_usage::{AccountHistory, AccountUsage};
        let history = AccountHistory {
            account_scope: "account-a".into(),
            observed_at: 100,
            through_day: test_day().to_string(),
            today: confirmed_empty_account_period(),
            month: confirmed_empty_account_period(),
            daily: vec![],
            coverage: "Cursor dashboard account history".into(),
            breakdown: None,
        };
        let prior = AccountUsage { history: Some(history.clone()), ..Default::default() };
        for (scope, error) in [
            (None, Some("Cursor response could not be read")),
            (Some("account-b"), Some("Cursor response could not be read")),
            (Some("account-a"), Some("Reconnect Cursor to read account usage.")),
            (Some("account-a"), Some("Cursor usage is unavailable (HTTP 404).")),
            (Some("account-a"), None),
        ] {
            let mut current = AccountUsage {
                account_scope: scope.map(str::to_owned),
                history_error: error.map(str::to_owned),
                ..Default::default()
            };
            retain_cursor_history(&mut current, Some(&prior));
            assert!(current.history.is_none(), "Do not retain history for {scope:?} / {error:?}");
        }
        for error in [
            "Cursor history pagination was incomplete",
            "Cursor history pagination was inconsistent",
            "Cursor usage is unavailable (HTTP 408).",
            "Cursor usage is unavailable (HTTP 503).",
            "Cursor is rate limited. Wait a few minutes before refreshing.",
        ] {
            let mut current = AccountUsage {
                account_scope: Some("account-a".into()),
                history_error: Some(error.into()),
                ..Default::default()
            };
            retain_cursor_history(&mut current, Some(&prior));
            assert_eq!(current.history.unwrap().observed_at, 100, "Retain history for {error}");
        }
        let mut current = AccountUsage {
            account_scope: Some("account-a".into()),
            history: Some(AccountHistory { observed_at: 200, ..history }),
            ..Default::default()
        };
        retain_cursor_history(&mut current, Some(&prior));
        assert_eq!(current.history.unwrap().observed_at, 200, "A confirmed-empty successful fetch replaces old history");
    }

    fn cached_cursor_account() -> CachedProvider {
        use crate::provider_usage::{AccountHistory, AccountUsage};
        CachedProvider {
            usage: Some(AccountUsage {
                account: Some("Account A".into()),
                account_scope: Some("scope-a".into()),
                observed_at: 100,
                history: Some(AccountHistory {
                    account_scope: "scope-a".into(),
                    observed_at: 100,
                    through_day: test_day().to_string(),
                    today: confirmed_empty_account_period(),
                    month: confirmed_empty_account_period(),
                    daily: vec![], coverage: "Cursor dashboard".into(), breakdown: None,
                }),
                ..Default::default()
            }),
            error: None,
        }
    }

    #[test]
    fn cursor_top_level_transient_failure_retains_stale_history_across_reopen() {
        use crate::provider_usage::AccountReadError;
        use bridge_protocol::messages::MenuBarProvider;
        let error = "Cursor usage request failed or timed out. Try Refresh.";
        let cache = cache_provider_result(MenuBarProvider::Cursor, cached_cursor_account(), Err(AccountReadError {
            message: error.into(), retry_account_scope: Some("scope-a".into()),
        }));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        {
            let db = crate::store::open(&path).unwrap();
            db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview','cursor',?1,'now','now')",
                [serde_json::to_string(&cache).unwrap()]).unwrap();
        }
        let db = crate::store::open(&path).unwrap();
        let reopened = load_provider(&db, "cursor").unwrap();
        assert_eq!(reopened.error.as_deref(), Some(error));
        let usage = reopened.usage.unwrap();
        assert_eq!(usage.account.as_deref(), Some("Account A"));
        assert_eq!(usage.observed_at, 100);
        let history = usage.history.unwrap();
        assert_eq!(history.observed_at, 100);
        let projected = project_account_history(history, test_day(), 101, usage.history_error.as_deref());
        assert_eq!(projected.month.cost_microusd.value, Some(0.0));
        assert_eq!(projected.month.cost_microusd.status, Status::Stale);
        assert!(projected.coverage.contains("timed out"));
    }

    #[test]
    fn cursor_top_level_failure_never_reuses_an_unverified_or_different_account() {
        use crate::provider_usage::AccountReadError;
        use bridge_protocol::messages::MenuBarProvider;
        for scope in [None, Some("scope-b"), Some("")] {
            let cache = cache_provider_result(MenuBarProvider::Cursor, cached_cursor_account(), Err(AccountReadError {
                message: "Fetch failed".into(), retry_account_scope: scope.map(str::to_owned),
            }));
            let usage = cache.usage.unwrap();
            assert!(usage.account.is_none());
            assert!(usage.account_scope.is_none());
            assert!(usage.history.is_none());
        }
        let mut prior = cached_cursor_account();
        prior.usage.as_mut().unwrap().history.as_mut().unwrap().account_scope = "scope-b".into();
        let cache = cache_provider_result(MenuBarProvider::Cursor, prior, Err(AccountReadError {
            message: "Fetch failed".into(), retry_account_scope: Some("scope-a".into()),
        }));
        assert!(cache.usage.unwrap().history.is_none());
        let fresh = cached_cursor_account().usage.unwrap();
        let cache = cache_provider_result(MenuBarProvider::Cursor, cached_cursor_account(), Ok(fresh));
        assert!(cache.error.is_none());
        assert!(cache.usage.unwrap().history_error.is_none());
    }

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
