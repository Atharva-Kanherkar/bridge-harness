use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bridge_protocol::messages::{UsageDailyOverview, UsageModelOverview, UsagePeriodOverview};
use chrono::TimeZone;
use rusqlite::{types::ValueRef, Connection, OpenFlags, OptionalExtension};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

const HISTORY_PAGE_SIZE: usize = 1_000;
const HISTORY_MAX_PAGES: usize = 50;
const HISTORY_DEADLINE: Duration = Duration::from_secs(20);

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
pub(super) fn read() -> Result<AccountUsage, AccountReadError> {
    let home = std::env::var_os("HOME").ok_or("Home directory unavailable")?;
    let path =
        Path::new(&home).join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
    let token = access_token(&path)?;
    let now = chrono::Utc::now().timestamp();
    let (subject, cookie) = session(&token, now)?;
    let client = http::client()?;
    let mut result = fetch_account(&client, &subject, &cookie, now, "https://cursor.com")?;
    // Grok Bot has a separate included allowance. Its optional endpoint must
    // never discard successfully fetched Cursor/Third Party account limits.
    result.windows.extend(fetch_grok_bot(&client, &cookie));
    // History is optional enrichment. A failure must not discard current account quotas,
    // and the central cache can retain history only for this verified account.
    match fetch_history(&client, &cookie, &subject, now) {
        Ok(history) => result.history = Some(history),
        Err(error) => result.history_error = Some(error),
    }
    Ok(result)
}

fn fetch_account(
    client: &reqwest::blocking::Client,
    subject: &str,
    cookie: &str,
    now: i64,
    base_url: &str,
) -> Result<AccountUsage, AccountReadError> {
    let scope = account_scope(subject);
    let account_request = http::secret(
        client.get(format!("{base_url}/api/auth/me"))
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(2)),
        true,
        cookie,
    )?;
    let usage_request = http::secret(
        client.get(format!("{base_url}/api/usage-summary"))
            .header("Accept", "application/json"),
        true,
        cookie,
    )?;
    // Like CodexBar's CursorStatusProbe, account metadata is optional. The
    // validated local session identifies the account; usage-summary determines
    // whether its usage is available. Fetch both together with a shorter bound
    // for the label lookup so its failure cannot block valid account limits.
    let (me, usage) = std::thread::scope(|threads| {
        let account = threads.spawn(|| http::json(account_request, "Cursor"));
        let usage = http::json_result(usage_request, "Cursor");
        let me = account.join().ok().and_then(Result::ok).unwrap_or(Value::Null);
        (me, usage)
    });
    // A returned mismatch still invalidates the account, even if the usage
    // request failed transiently. Never retain another account's cache.
    if let Some(actual) = me["sub"].as_str() {
        if actual.rsplit('|').next().map(str::to_lowercase)
            != subject.rsplit('|').next().map(str::to_lowercase)
        {
            return Err("Cursor account changed. Sign in again in Cursor desktop.".into());
        }
    }
    let usage = usage.map_err(|error| AccountReadError::for_account(error, &scope))?;
    let mut result = parse(&usage, now)?;
    result.account = public_text(&me["email"]).or_else(|| public_text(&me["sub"]));
    result.account_scope = Some(scope);
    Ok(result)
}

// Adapted from CodexBar's CursorSandUsage and CursorStatusProbe at 928166f.
// Copyright (c) 2026 Peter Steinberger; see docs/third-party/CodexBar-LICENSE.txt.
fn grok_bot_request(
    client: &reqwest::blocking::Client,
    cookie: &str,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    http::secret(
        client
            .post("https://cursor.com/api/dashboard/get-sand-usage-status")
            .timeout(Duration::from_secs(5))
            .header("Origin", "https://cursor.com")
            .header("Accept", "application/json")
            .json(&json!({})),
        true,
        cookie,
    )
}

fn fetch_grok_bot(client: &reqwest::blocking::Client, cookie: &str) -> Option<UsageQuotaWindow> {
    grok_bot_response(
        grok_bot_request(client, cookie).and_then(|request| http::json(request, "Cursor")),
    )
}

fn grok_bot_response(response: Result<Value, String>) -> Option<UsageQuotaWindow> {
    let value = response.ok()?;
    if value["hasNonZeroIncludedLimit"].as_bool() != Some(true) {
        return None;
    }
    let percent = value["usagePercent"]
        .as_f64()
        .filter(|v| v.is_finite())?
        .clamp(0.0, 100.0);
    let reset = timestamp(&value["nextResetTimestampUtc"]);
    let minutes = timestamp(&value["currentPeriodStart"])
        .zip(reset)
        .and_then(|(start, end)| end.checked_sub(start))
        .map(|seconds| (seconds as f64 / 60.0).round() as i64)
        .filter(|minutes| *minutes > 0);
    Some(window(
        "cursor-grok-bot",
        "Grok Bot",
        Some(percent),
        reset,
        minutes,
    ))
}

#[derive(Clone, Debug, PartialEq)]
struct CursorEvent {
    timestamp_ms: i64,
    model: String,
    input: i64,
    output: i64,
    cache_write: i64,
    cache_read: i64,
    total_cents: Option<f64>,
    cost_invalid: bool,
}

fn finite_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite())
}
fn nonnegative_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| (0..=1_000_000_000_000_000).contains(v))
}
fn token_count(tokens: &serde_json::Map<String, Value>, key: &str) -> Option<i64> {
    match tokens.get(key) {
        None | Some(Value::Null) => Some(0),
        Some(value) => nonnegative_i64(value),
    }
}
fn event(value: &Value) -> Result<Option<CursorEvent>, String> {
    let timestamp_ms = value["timestamp"]
        .as_i64()
        .or_else(|| value["timestamp"].as_str().and_then(|v| v.parse().ok()))
        .ok_or("Cursor history event has an invalid timestamp")?;
    if timestamp_ms <= 0 {
        return Err("Cursor history event has an invalid timestamp".into());
    }
    let Some(tokens) = value.get("tokenUsage").and_then(Value::as_object) else {
        return Ok(None);
    };
    let input = token_count(tokens, "inputTokens")
        .ok_or("Cursor history event has invalid input tokens")?;
    let output = token_count(tokens, "outputTokens")
        .ok_or("Cursor history event has invalid output tokens")?;
    let cache_write = token_count(tokens, "cacheWriteTokens")
        .ok_or("Cursor history event has invalid cache-write tokens")?;
    let cache_read = token_count(tokens, "cacheReadTokens")
        .ok_or("Cursor history event has invalid cache-read tokens")?;
    let total = input
        .checked_add(output)
        .and_then(|value| value.checked_add(cache_write))
        .and_then(|value| value.checked_add(cache_read))
        .ok_or("Cursor history event token total overflowed")?;
    if total == 0 {
        return Ok(None);
    }
    let cost_value = tokens.get("totalCents");
    let total_cents = cost_value
        .and_then(finite_number)
        .filter(|v| (0.0..=1_000_000_000.0).contains(v));
    let cost_invalid = cost_value.is_some_and(|v| !v.is_null()) && total_cents.is_none();
    Ok(Some(CursorEvent {
        timestamp_ms,
        model: public_text(&value["model"]).unwrap_or_else(|| "unknown".into()),
        input,
        output,
        cache_write,
        cache_read,
        total_cents,
        cost_invalid,
    }))
}

fn page(value: Value) -> Result<(Option<usize>, Vec<Value>), String> {
    let object = value
        .as_object()
        .ok_or("Cursor history returned an unrecognized response")?;
    if object.is_empty() {
        return Ok((Some(0), vec![]));
    }
    let count = match object.get("totalUsageEventsCount") {
        Some(value) => Some(
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("Cursor history returned an invalid event count")?,
        ),
        None => None,
    };
    let events = match object.get("usageEventsDisplay") {
        Some(Value::Array(events)) => events.clone(),
        None if count.is_some() => vec![],
        _ => return Err("Cursor history returned an unrecognized response".into()),
    };
    Ok((count, events))
}

fn boundary_overlap(previous: &[Value], current: &[Value]) -> usize {
    (1..=previous.len().min(current.len()))
        .rev()
        .find(|count| previous[previous.len() - count..] == current[..*count])
        .unwrap_or(0)
}

fn reconcile_pages(
    pages: Vec<Vec<Value>>,
    expected: Option<usize>,
    completed: bool,
) -> Result<Vec<Value>, String> {
    let raw_count: usize = pages.iter().map(Vec::len).sum();
    if !completed || expected.is_some_and(|total| raw_count < total) {
        return Err("Cursor history pagination was incomplete".into());
    }
    let Some(expected) = expected else {
        return Ok(pages.into_iter().flatten().collect());
    };
    if raw_count == expected {
        return Ok(pages.into_iter().flatten().collect());
    }
    let mut removals = raw_count.saturating_sub(expected);
    let mut result = pages.first().cloned().unwrap_or_default();
    for index in 1..pages.len() {
        let remove = boundary_overlap(&pages[index - 1], &pages[index]).min(removals);
        result.extend(pages[index].iter().skip(remove).cloned());
        removals -= remove;
    }
    if removals != 0 || result.len() != expected {
        return Err("Cursor history pagination was inconsistent".into());
    }
    Ok(result)
}

fn events_in_window(raw: &[Value], start: i64, end: i64) -> Result<Vec<CursorEvent>, String> {
    let mut events = Vec::new();
    for value in raw {
        let timestamp = value["timestamp"]
            .as_i64()
            .or_else(|| value["timestamp"].as_str().and_then(|v| v.parse().ok()));
        if timestamp.is_some_and(|at| at < start || at > end) {
            continue;
        }
        if let Some(row) = event(value)? {
            events.push(row);
        }
    }
    Ok(events)
}

fn fetch_history(
    client: &reqwest::blocking::Client,
    cookie: &str,
    subject: &str,
    now: i64,
) -> Result<AccountHistory, String> {
    let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    let tz: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::UTC);
    let now_dt =
        chrono::DateTime::from_timestamp(now, 0).ok_or("Cursor history clock is unavailable")?;
    let today = now_dt.with_timezone(&tz).date_naive();
    let start_day = today - chrono::Duration::days(29);
    let start = tz
        .from_local_datetime(&start_day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .ok_or("Cursor history time zone is unavailable")?
        .timestamp_millis();
    let end = now_dt.timestamp_millis();
    let started = Instant::now();
    let mut pages = Vec::new();
    let mut expected = None;
    let mut completed = false;
    for page_number in 1..=HISTORY_MAX_PAGES {
        let remaining = HISTORY_DEADLINE.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("Cursor history request timed out".into());
        }
        let request = client.post("https://cursor.com/api/dashboard/get-filtered-usage-events")
            .timeout(remaining)
            .header("Origin", "https://cursor.com")
            .json(&json!({"page": page_number, "pageSize": HISTORY_PAGE_SIZE, "startDate": start.to_string(), "endDate": end.to_string()}));
        let value = http::json(http::secret(request, true, cookie)?, "Cursor")?;
        let (count, rows) = page(value)?;
        if let Some(count) = count {
            if expected.is_some_and(|old| old != count) {
                return Err("Cursor history pagination was inconsistent".into());
            }
            expected = Some(count);
        }
        let short = rows.len() < HISTORY_PAGE_SIZE;
        pages.push(rows);
        if short {
            completed = true;
            break;
        }
    }
    let raw = reconcile_pages(pages, expected, completed)?;
    let events = events_in_window(&raw, start, end)?;
    Ok(history(events, subject, today, tz, now))
}

#[derive(Default, Clone)]
struct Aggregate {
    input: i64,
    output: i64,
    cache: i64,
    cache_read: i64,
    cache_write: i64,
    records: i64,
    unpriced_records: i64,
    cost_cents: f64,
    cost_known: bool,
    unpriced: bool,
    invalid: bool,
}
impl Aggregate {
    fn add(&mut self, event: &CursorEvent) {
        let Some(input) = self.input.checked_add(event.input) else {
            self.invalid = true;
            return;
        };
        let Some(output) = self.output.checked_add(event.output) else {
            self.invalid = true;
            return;
        };
        let Some(cache) = self
            .cache
            .checked_add(event.cache_read)
            .and_then(|v| v.checked_add(event.cache_write))
        else {
            self.invalid = true;
            return;
        };
        self.input = input;
        self.output = output;
        self.cache = cache;
        // The combined cache sum was checked above; each component fits too.
        self.cache_read += event.cache_read;
        self.cache_write += event.cache_write;
        self.records += 1;
        if event.cost_invalid || event.total_cents.is_none() {
            self.unpriced = true;
            self.unpriced_records += 1;
        }
        if let Some(cents) = event.total_cents {
            let next = self.cost_cents + cents;
            if !next.is_finite() || next * 10_000.0 > i64::MAX as f64 {
                self.unpriced = true;
                self.unpriced_records += 1;
            } else {
                self.cost_cents = next;
                self.cost_known = true;
            }
        }
    }
    fn period(&self, models: Vec<UsageModelOverview>) -> UsagePeriodOverview {
        let known = |value| UsageMetric::known(value as f64, UsageMetricSource::Reported);
        UsagePeriodOverview {
            tokens: if self.invalid {
                UsageMetric::unavailable()
            } else {
                self.input
                    .checked_add(self.output)
                    .and_then(|v| v.checked_add(self.cache))
                    .map(known)
                    .unwrap_or_else(UsageMetric::unavailable)
            },
            cost_microusd: if self.invalid || self.unpriced || !self.cost_known {
                UsageMetric::unavailable()
            } else {
                UsageMetric::known(
                    (self.cost_cents * 10_000.0).round(),
                    UsageMetricSource::Reported,
                )
            },
            models,
        }
    }
    fn merge(&mut self, other: &Aggregate) {
        self.invalid |= other.invalid;
        let Some(input) = self.input.checked_add(other.input) else {
            self.invalid = true;
            return;
        };
        let Some(output) = self.output.checked_add(other.output) else {
            self.invalid = true;
            return;
        };
        let Some(cache) = self.cache.checked_add(other.cache) else {
            self.invalid = true;
            return;
        };
        let cost = self.cost_cents + other.cost_cents;
        if !cost.is_finite() || cost * 10_000.0 > i64::MAX as f64 {
            self.invalid = true;
            return;
        }
        self.input = input;
        self.output = output;
        self.cache = cache;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.records += other.records;
        self.unpriced_records += other.unpriced_records;
        self.cost_cents = cost;
        self.cost_known |= other.cost_known;
        self.unpriced |= other.unpriced;
    }

    fn bucket(&self, day: &str, model: &str) -> Option<crate::usage_summary::UsageBucket> {
        use crate::usage_pricing::CostSource;
        use crate::usage_summary::{UsageBucket, UsageBucketTotals};
        self.input.checked_add(self.output)?.checked_add(self.cache)?;
        (!self.invalid).then(|| UsageBucket {
            day: day.into(),
            hour_start: None,
            harness: "cursor".into(),
            model: model.into(),
            totals: UsageBucketTotals {
                uncached_input_tokens: self.input,
                cache_read_tokens: self.cache_read,
                cache_write_tokens: self.cache_write,
                output_tokens: self.output,
                reasoning_tokens: 0,
            },
            // Sum fractional cents before rounding, like the menu collector.
            cost_microusd: (self.cost_cents * 10_000.0).round() as i64,
            cache_savings_microusd: 0,
            cost_source: if self.unpriced { CostSource::Unpriced } else { CostSource::ProviderReported },
            records: self.records,
            unpriced_records: self.unpriced_records,
            sessions: None,
        })
    }
}

fn history(
    events: Vec<CursorEvent>,
    subject: &str,
    today: chrono::NaiveDate,
    tz: chrono_tz::Tz,
    now: i64,
) -> AccountHistory {
    let mut grouped: BTreeMap<String, BTreeMap<String, Aggregate>> = BTreeMap::new();
    let mut month_models: BTreeMap<String, Aggregate> = BTreeMap::new();
    for event in &events {
        let Some(at) = chrono::DateTime::from_timestamp_millis(event.timestamp_ms) else {
            continue;
        };
        let day = at.with_timezone(&tz).date_naive().to_string();
        grouped
            .entry(day)
            .or_default()
            .entry(event.model.clone())
            .or_default()
            .add(event);
        month_models
            .entry(event.model.clone())
            .or_default()
            .add(event);
    }
    let mut daily = Vec::new();
    let mut buckets = Vec::new();
    let mut breakdown_complete = true;
    let mut month = Aggregate::default();
    let mut today_total = Aggregate::default();
    let mut found_today = false;
    for (day, models) in grouped {
        let mut day_total = Aggregate::default();
        let mut model_rows = Vec::new();
        for (model, aggregate) in models {
            if let Some(bucket) = aggregate.bucket(&day, &model) {
                buckets.push(bucket);
            } else {
                breakdown_complete = false;
            }
            model_rows.push(model_row(model, &aggregate));
            day_total.merge(&aggregate);
        }
        model_rows.sort_by(|a, b| {
            b.total_tokens
                .value
                .partial_cmp(&a.total_tokens.value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if day == today.to_string() {
            found_today = true;
            today_total = day_total.clone();
        }
        month.merge(&day_total);
        daily.push(UsageDailyOverview {
            day,
            usage: day_total.period(model_rows),
        });
    }
    let empty = events.is_empty();
    if empty {
        month.cost_known = true;
    }
    if !found_today {
        today_total.cost_known = true;
    }
    let month_rows = month_models
        .into_iter()
        .map(|(model, aggregate)| model_row(model, &aggregate))
        .collect();
    AccountHistory {
        account_scope: account_scope(subject),
        observed_at: now,
        through_day: today.to_string(),
        today: today_total.period(
            daily
                .iter()
                .find(|d| d.day == today.to_string())
                .map(|d| d.usage.models.clone())
                .unwrap_or_default(),
        ),
        month: month.period(month_rows),
        daily,
        breakdown: breakdown_complete.then(|| AccountHistoryBreakdown {
            time_zone: tz.name().into(),
            since_day: (today - chrono::Duration::days(29)).to_string(),
            buckets,
        }),
        coverage: if empty {
            "Cursor dashboard account history · confirmed empty for the last 30 days".into()
        } else {
            "Cursor dashboard account history · last 30 days · API-rate cost, distinct from actual plan billing".into()
        },
    }
}

fn account_scope(subject: &str) -> String {
    format!("{:x}", Sha256::digest(subject.as_bytes()))
}

fn model_row(model: String, aggregate: &Aggregate) -> UsageModelOverview {
    let metric = |value| {
        if aggregate.invalid {
            UsageMetric::unavailable()
        } else {
            UsageMetric::known(value as f64, UsageMetricSource::Reported)
        }
    };
    let total = aggregate
        .input
        .checked_add(aggregate.output)
        .and_then(|v| v.checked_add(aggregate.cache));
    UsageModelOverview {
        model,
        input_tokens: metric(aggregate.input),
        output_tokens: metric(aggregate.output),
        cache_tokens: metric(aggregate.cache),
        total_tokens: total.map(metric).unwrap_or_else(UsageMetric::unavailable),
        cost_microusd: if aggregate.invalid || aggregate.unpriced || !aggregate.cost_known {
            UsageMetric::unavailable()
        } else {
            UsageMetric::known(
                (aggregate.cost_cents * 10_000.0).round(),
                UsageMetricSource::Reported,
            )
        },
    }
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

    const ACCOUNT: &str = r#"{"sub":"auth0|account-a","email":"fixture@example.test"}"#;
    const SUMMARY: &str = r#"{"individualUsage":{"plan":{"totalPercentUsed":35}}}"#;

    fn account_probe(
        account: (u16, &str, Duration),
        usage: (u16, &str, Duration),
    ) -> Result<AccountUsage, AccountReadError> {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let owned = |(code, body, delay): (u16, &str, Duration)| (code, body.to_owned(), delay);
        let account = owned(account);
        let usage = owned(usage);
        let server = std::thread::spawn(move || std::thread::scope(|threads| {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let until = Instant::now() + Duration::from_secs(4);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        error => panic!("missing fixture request: {error:?}"),
                    }
                };
                let account = &account;
                let usage = &usage;
                requests.push(threads.spawn(move || {
                    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut chunk = [0; 1024];
                        let count = socket.read(&mut chunk).unwrap();
                        assert!(count > 0 && request.len() < 4096);
                        request.extend_from_slice(&chunk[..count]);
                    }
                    let request = String::from_utf8(request).unwrap().to_lowercase();
                    assert!(request.contains("\r\ncookie: fixture-session\r\n"));
                    assert!(request.contains("\r\naccept: application/json\r\n"));
                    let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    let (code, body, delay) = match path {
                        "/api/auth/me" => account,
                        "/api/usage-summary" => usage,
                        _ => panic!("unexpected fixture path: {path}"),
                    };
                    std::thread::sleep(*delay);
                    let _ = write!(socket, "HTTP/1.1 {code} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    path.to_owned()
                }));
            }
            let mut paths: Vec<_> = requests.into_iter().map(|request| request.join().unwrap()).collect();
            paths.sort();
            assert_eq!(paths, ["/api/auth/me", "/api/usage-summary"]);
        }));
        let client = reqwest::blocking::Client::builder().no_proxy()
            .timeout(Duration::from_secs(1)).build().unwrap();
        let result = fetch_account(&client, "auth0|account-a", "fixture-session", 100, &url);
        server.join().unwrap();
        result
    }

    #[test]
    fn cursor_usage_survives_missing_or_failed_optional_account_metadata() {
        for (code, body, delay) in [
            (401, "{}", Duration::ZERO),
            (503, "{}", Duration::ZERO),
            (200, "not-json", Duration::ZERO),
            (204, "", Duration::ZERO),
            (200, "{}", Duration::ZERO),
            (200, ACCOUNT, Duration::from_millis(2200)),
        ] {
            let result = account_probe((code, body, delay), (200, SUMMARY, Duration::ZERO)).unwrap();
            assert_eq!(result.windows[0].used_percent.value, Some(35.0));
            assert_eq!(result.windows[0].used_percent.source, Some(UsageMetricSource::Reported));
            assert_eq!(result.observed_at, 100);
            assert_eq!(result.account_scope, Some(account_scope("auth0|account-a")));
            assert!(result.account.is_none());
        }
        let result = account_probe((200, ACCOUNT, Duration::ZERO), (200, SUMMARY, Duration::ZERO)).unwrap();
        assert_eq!(result.account.as_deref(), Some("fixture@example.test"));
    }

    #[test]
    fn cursor_usage_summary_timeouts_and_server_failures_keep_the_current_account_scope() {
        for (code, body, delay) in [
            (200, SUMMARY, Duration::from_millis(1200)),
            (503, "{}", Duration::ZERO),
        ] {
            let error = account_probe((200, ACCOUNT, Duration::ZERO), (code, body, delay)).unwrap_err();
            assert_eq!(error.retry_account_scope, Some(account_scope("auth0|account-a")));
            if !delay.is_zero() {
                assert!(error.message.contains("timed out"));
            }
        }
    }

    #[test]
    fn cursor_rejected_usage_and_malformed_summary_do_not_authorize_stale_history() {
        for (code, body) in [(401, "{}"), (403, "{}"), (200, "not-json"), (200, "{}")] {
            let error = account_probe((200, ACCOUNT, Duration::ZERO), (code, body, Duration::ZERO)).unwrap_err();
            assert!(error.retry_account_scope.is_none(), "{code}: {body}");
        }
    }

    #[test]
    fn cursor_returned_account_mismatch_rejects_usage_and_cache_reuse() {
        for (code, body) in [(200, SUMMARY), (503, "{}")] {
            let error = account_probe(
                (200, r#"{"sub":"auth0|different-account"}"#, Duration::ZERO),
                (code, body, Duration::ZERO),
            ).unwrap_err();
            assert!(error.retry_account_scope.is_none());
            assert!(error.message.contains("account changed"));
        }
    }

    #[test]
    fn grok_bot_uses_its_own_allowance_and_reset() {
        let row = grok_bot_response(Ok(json!({
            "currentPeriodStart":"2026-08-17T07:57:50.647Z",
            "nextResetTimestampUtc":"2026-08-24T07:57:50.647Z",
            "usagePercent":35, "hasAvailableUsage":false, "hasNonZeroIncludedLimit":true
        })))
        .unwrap();
        assert_eq!(row.id, "cursor-grok-bot");
        assert_eq!(row.label, "Grok Bot");
        assert_eq!(row.used_percent.value, Some(35.0));
        assert_eq!(row.window_minutes, Some(10080));
        assert_eq!(row.resets_at, timestamp(&json!("2026-08-24T07:57:50.647Z")));
        for (raw, expected) in [(0.0, 0.0), (-1.0, 0.0), (150.0, 100.0)] {
            let row = grok_bot_response(Ok(
                json!({"usagePercent":raw, "hasNonZeroIncludedLimit":true}),
            ))
            .unwrap();
            assert_eq!(row.used_percent.value, Some(expected));
            assert!(row.resets_at.is_none() && row.window_minutes.is_none());
        }
    }

    #[test]
    fn optional_grok_bot_failure_never_replaces_base_cursor_usage() {
        for response in [
            Err("timeout".into()),
            Ok(json!({})),
            Ok(json!({"usagePercent":35, "hasNonZeroIncludedLimit":false})),
            Ok(json!({"hasNonZeroIncludedLimit":true})),
            Ok(json!({"usagePercent":"bad", "hasNonZeroIncludedLimit":true})),
        ] {
            let mut base = parse(
                &json!({"individualUsage":{"plan":{"totalPercentUsed":25}}}),
                10,
            )
            .unwrap();
            base.windows.extend(grok_bot_response(response));
            assert_eq!(base.windows.len(), 1);
            assert_eq!(base.windows[0].used_percent.value, Some(25.0));
        }
        let malformed_reset = grok_bot_response(Ok(json!({"usagePercent":35,
            "hasNonZeroIncludedLimit":true, "currentPeriodStart":"bad", "nextResetTimestampUtc":"bad"}))).unwrap();
        assert!(malformed_reset.resets_at.is_none() && malformed_reset.window_minutes.is_none());
    }

    #[test]
    fn grok_bot_request_is_bounded_and_uses_the_existing_cursor_session() {
        let request = grok_bot_request(&http::client().unwrap(), "test=session")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://cursor.com/api/dashboard/get-sand-usage-status"
        );
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(request.timeout(), Some(&Duration::from_secs(5)));
        assert_eq!(request.headers()["origin"], "https://cursor.com");
        assert_eq!(request.headers()["accept"], "application/json");
        assert_eq!(request.headers()["content-type"], "application/json");
        assert_eq!(request.headers()["cookie"], "test=session");
        assert!(request.headers()["cookie"].is_sensitive());
        assert_eq!(request.body().unwrap().as_bytes(), Some(b"{}".as_slice()));
    }

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
    fn account_history_groups_models_and_keeps_unpriced_cost_unavailable() {
        let tz = chrono_tz::UTC;
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-09-10T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        let priced = event(&json!({"timestamp":at.to_string(),"model":"cursor-model","tokenUsage":{"inputTokens":"10","outputTokens":2,"cacheReadTokens":3,"cacheWriteTokens":4,"totalCents":"1.25"}})).unwrap().unwrap();
        let unpriced = event(&json!({"timestamp":at,"model":"other","tokenUsage":{"inputTokens":1,"outputTokens":1}})).unwrap().unwrap();
        let report = history(
            vec![priced, unpriced],
            "auth0|account-a",
            today,
            tz,
            at / 1000,
        );
        assert_eq!(report.daily.len(), 1);
        assert_eq!(report.today.tokens.value, Some(21.0));
        assert_eq!(report.month.models.len(), 2);
        let exact = report.breakdown.as_ref().unwrap();
        assert_eq!(exact.time_zone, "UTC");
        assert_eq!(exact.since_day, "2026-08-12");
        let bucket = exact.buckets.iter().find(|row| row.model == "cursor-model").unwrap();
        assert_eq!(bucket.totals.uncached_input_tokens, 10);
        assert_eq!(bucket.totals.cache_read_tokens, 3);
        assert_eq!(bucket.totals.cache_write_tokens, 4);
        assert_eq!(bucket.totals.output_tokens, 2);
        assert_eq!(bucket.cost_microusd, 12_500);
        assert_eq!(bucket.records, 1);
        assert_eq!(bucket.sessions, None);
        let unknown = exact.buckets.iter().find(|row| row.model == "other").unwrap();
        assert_eq!(unknown.unpriced_records, 1);
        assert_eq!(unknown.cost_source, crate::usage_pricing::CostSource::Unpriced);
        assert_eq!(
            report.month.cost_microusd.value, None,
            "a mixed priced/unpriced total is not a lower bound"
        );
        assert_eq!(
            report.daily[0]
                .usage
                .models
                .iter()
                .find(|m| m.model == "cursor-model")
                .unwrap()
                .cost_microusd
                .value,
            Some(12_500.0)
        );
        let other = history(vec![], "auth0|account-b", today, tz, at / 1000);
        assert_ne!(report.account_scope, other.account_scope);
        assert_eq!(other.today.tokens.value, Some(0.0));
        assert_eq!(other.today.cost_microusd.value, Some(0.0));
        assert!(other.breakdown.unwrap().buckets.is_empty());
    }

    #[test]
    fn exact_dashboard_buckets_keep_known_subtotals_and_fractional_costs() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
        let at = day.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let row = |cents| event(&json!({"timestamp":at * 1000,"model":"same-model","tokenUsage":{"inputTokens":1,"totalCents":cents}})).unwrap().unwrap();
        let report = history(vec![row(json!(0.00004)), row(json!(0.00004)), row(Value::Null)], "account", day, chrono_tz::UTC, at);
        let bucket = &report.breakdown.as_ref().unwrap().buckets[0];
        assert_eq!(bucket.cost_microusd, 1, "round the cell sum once, not each event");
        assert_eq!(bucket.records, 3);
        assert_eq!(bucket.unpriced_records, 1);
        assert_eq!(bucket.cost_source, crate::usage_pricing::CostSource::Unpriced);
        assert_eq!(report.month.cost_microusd.value, None);
    }

    #[test]
    fn pagination_requires_completion_and_reconciles_only_proven_boundary_overlap() {
        let a = json!({"timestamp":"1"});
        let b = json!({"timestamp":"2"});
        let c = json!({"timestamp":"3"});
        assert_eq!(
            reconcile_pages(
                vec![vec![a.clone(), b.clone()], vec![b, c.clone()]],
                Some(3),
                true
            )
            .unwrap()
            .len(),
            3
        );
        assert!(reconcile_pages(vec![vec![a.clone()]], Some(2), true).is_err());
        assert!(reconcile_pages(vec![vec![a]], Some(1), false).is_err());
        assert!(page(json!({"error":"unauthorized"})).is_err());
        assert_eq!(page(json!({})).unwrap(), (Some(0), vec![]));
    }

    #[test]
    fn malformed_or_negative_event_numbers_are_not_presented_as_usage() {
        assert!(event(&json!({"timestamp":1,"model":"x","tokenUsage":{"inputTokens":-1,"outputTokens":2,"totalCents":1}})).is_err());
        let row = event(
            &json!({"timestamp":1,"model":"x","tokenUsage":{"inputTokens":1,"totalCents":"bad"}}),
        )
        .unwrap()
        .unwrap();
        assert!(row.cost_invalid);
        assert_eq!(row.total_cents, None);
    }

    #[test]
    fn history_filters_outside_window_and_rounds_fractional_cost_once() {
        let row = |timestamp, cents| json!({"timestamp":timestamp,"model":"x","tokenUsage":{"inputTokens":1,"totalCents":cents}});
        let events = events_in_window(
            &[
                row(99, 0.00004),
                row(100, 0.00004),
                row(101, 0.00004),
                row(102, 0.00004),
            ],
            100,
            101,
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        let today = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let report = history(events, "subject", today, chrono_tz::UTC, 1);
        assert_eq!(report.month.tokens.value, Some(2.0));
        assert_eq!(report.month.cost_microusd.value, Some(1.0));
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
