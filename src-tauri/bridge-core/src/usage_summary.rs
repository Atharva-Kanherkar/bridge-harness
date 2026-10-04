//! Day and hour roll-ups of the usage ledger, per harness and model.
//!
//! Two record sources feed one aggregation: Bridge's own `usage_ledger` rows
//! (live) and the `agent_usage_observations` an importer wrote from a
//! provider's transcripts (imported). They describe the same provider
//! sessions from two vantage points, so when both are asked for, a complete
//! imported source's observations replace matching Bridge rows. Partial
//! sources stay out of totals until their bounded rebuild finishes, avoiding
//! a half-imported transcript suppressing a complete live session. A Bridge
//! session whose `provider_session_id` matches an imported
//! `native_session_id` counts once: the transcript is the complete record and
//! wins, and the live rows it displaces are reported in `duplicates_dropped`.
//!
//! Costs are re-priced at read time so a new override or a refreshed rate
//! table applies to history. A row the provider priced keeps its figure; a
//! bucket's `cost_source` is the weakest provenance among its rows.

use crate::analytics::{ExactTotalFormula, TokenUsage};
use crate::usage_pricing::{CostSource, Pricing, PricingStatus};
use crate::BridgeError;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UsageResolution {
    #[default]
    Day,
    Hour,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummaryRequest {
    /// Inclusive first day, `YYYY-MM-DD` in `time_zone`.
    pub since_day: String,
    /// Inclusive last day, `YYYY-MM-DD` in `time_zone`.
    pub until_day: String,
    pub resolution: UsageResolution,
    /// IANA zone the caller lives in; UTC when absent.
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub include_imported: bool,
    #[serde(default)]
    pub include_dashboard: bool,
    /// Exact UTC bounds, required for hour resolution: inclusive start and
    /// exclusive end, at most 24 hours apart.
    #[serde(default)]
    pub since_time: Option<String>,
    #[serde(default)]
    pub until_time: Option<String>,
}

/// Mutually exclusive token buckets summed over a cell. `reasoning_tokens` is
/// a breakdown of `output_tokens`, never an addend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucketTotals {
    pub uncached_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
}

impl UsageBucketTotals {
    fn add(&mut self, tokens: &TokenUsage) {
        self.uncached_input_tokens += tokens.uncached_input_tokens.unwrap_or(0).max(0);
        self.cache_read_tokens += tokens.cache_read_tokens.unwrap_or(0).max(0);
        self.cache_write_tokens += tokens.cache_write_tokens.unwrap_or(0).max(0);
        self.output_tokens += tokens.output_tokens.unwrap_or(0).max(0);
        self.reasoning_tokens += tokens.reasoning_tokens.unwrap_or(0).max(0);
    }
}

/// One `(day, hour_start?, harness, model)` cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub day: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hour_start: Option<String>,
    pub harness: String,
    pub model: String,
    pub totals: UsageBucketTotals,
    pub cost_microusd: i64,
    pub cache_savings_microusd: i64,
    pub cost_source: CostSource,
    pub records: i64,
    pub unpriced_records: i64,
    pub sessions: Option<i64>,
}

/// A history importer's standing, read from `agent_usage_sources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummarySource {
    pub id: String,
    #[serde(default)]
    pub origin: bridge_protocol::messages::UsageHistoryOrigin,
    pub agent: String,
    pub provider: String,
    pub coverage_state: String,
    pub coverage_reason: Option<String>,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub last_successful_scan_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub since_day: String,
    pub until_day: String,
    pub time_zone: String,
    pub resolution: UsageResolution,
    pub buckets: Vec<UsageBucket>,
    pub sources: Vec<UsageSummarySource>,
    pub pricing: PricingStatus,
    pub scan_duration_ms: i64,
    /// Live rows set aside because an imported transcript covers their
    /// session.
    pub duplicates_dropped: i64,
    pub live_records: i64,
    pub imported_records: i64,
}

/// One record as the aggregator sees it, whichever table it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageInput {
    pub occurred_at: DateTime<Utc>,
    pub harness: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub tokens: TokenUsage,
    pub reported_cost_microusd: Option<i64>,
    /// The row names its input but never recorded how much of it was cached.
    /// Cached input costs a tenth of the full rate and is most of an agentic
    /// request, so a full-rate price would overstate the row several times
    /// over; such a row counts its tokens and stays unpriced.
    pub cache_split_unknown: bool,
}

/// The validated shape of a request: parsed zone, day range, and hour bounds.
struct Window {
    zone: Tz,
    since_day: NaiveDate,
    until_day: NaiveDate,
    hours: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

impl Window {
    fn parse(request: &UsageSummaryRequest) -> Result<Self, BridgeError> {
        let zone: Tz = match request.time_zone.as_deref().map(str::trim).filter(|zone| !zone.is_empty()) {
            None => chrono_tz::UTC,
            Some(name) => name
                .parse()
                .map_err(|_| BridgeError::Invalid(format!("unknown time zone {name:?}")))?,
        };
        let day = |value: &str, field: &str| {
            NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
                .map_err(|_| BridgeError::Invalid(format!("{field} must be YYYY-MM-DD, got {value:?}")))
        };
        let since_day = day(&request.since_day, "sinceDay")?;
        let until_day = day(&request.until_day, "untilDay")?;
        if until_day < since_day {
            return Err(BridgeError::Invalid("untilDay is before sinceDay".into()));
        }
        let hours = match request.resolution {
            UsageResolution::Day => None,
            UsageResolution::Hour => {
                let (Some(since), Some(until)) = (&request.since_time, &request.until_time) else {
                    return Err(BridgeError::Invalid(
                        "hour resolution needs exact sinceTime and untilTime bounds".into(),
                    ));
                };
                let instant = |value: &str, field: &str| {
                    DateTime::parse_from_rfc3339(value.trim())
                        .map(|time| time.with_timezone(&Utc))
                        .map_err(|_| BridgeError::Invalid(format!("{field} must be RFC 3339, got {value:?}")))
                };
                let since = instant(since, "sinceTime")?;
                let until = instant(until, "untilTime")?;
                if until <= since {
                    return Err(BridgeError::Invalid("untilTime must be after sinceTime".into()));
                }
                if until - since > Duration::hours(24) {
                    return Err(BridgeError::Invalid(
                        "hour resolution covers at most 24 hours per request".into(),
                    ));
                }
                Some((since, until))
            }
        };
        Ok(Self { zone, since_day, until_day, hours })
    }

    fn local_day(&self, instant: DateTime<Utc>) -> NaiveDate {
        instant.with_timezone(&self.zone).date_naive()
    }

    /// A generous UTC envelope for the SQL prefilter; the exact test happens
    /// in Rust after parsing each timestamp.
    fn coarse_bounds(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        match self.hours {
            Some((since, until)) => (since, until),
            None => {
                let start = self.since_day.and_hms_opt(0, 0, 0).unwrap();
                let end = self.until_day.succ_opt().unwrap_or(self.until_day).and_hms_opt(0, 0, 0).unwrap();
                (
                    Utc.from_utc_datetime(&start) - Duration::days(1),
                    Utc.from_utc_datetime(&end) + Duration::days(1),
                )
            }
        }
    }
}

#[derive(Default)]
struct MutableBucket {
    totals: UsageBucketTotals,
    cost_microusd: i64,
    cache_savings_microusd: i64,
    records: i64,
    unpriced_records: i64,
    provider_reported_records: i64,
    sessions: BTreeSet<String>,
}

/// A bucket mixes rows of one model whose provenance can differ. The weakest
/// wins so the figure never overstates confidence.
fn resolve_cost_source(bucket: &MutableBucket) -> CostSource {
    if bucket.unpriced_records == bucket.records {
        CostSource::Unpriced
    } else if bucket.provider_reported_records == bucket.records {
        CostSource::ProviderReported
    } else {
        CostSource::ModelPriced
    }
}

/// Folds records into sorted buckets. Pure: callers own the queries.
pub fn aggregate(
    request: &UsageSummaryRequest,
    pricing: &Pricing,
    inputs: impl IntoIterator<Item = UsageInput>,
) -> Result<Vec<UsageBucket>, BridgeError> {
    let window = Window::parse(request)?;
    let mut buckets: BTreeMap<(String, String, String, String), MutableBucket> = BTreeMap::new();
    for input in inputs {
        let hour_start = match window.hours {
            Some((since, until)) => {
                if input.occurred_at < since || input.occurred_at >= until {
                    continue;
                }
                let elapsed = (input.occurred_at - since).num_seconds().max(0) / 3600;
                Some((since + Duration::hours(elapsed)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            }
            None => None,
        };
        let day = window.local_day(input.occurred_at);
        if window.hours.is_none() && (day < window.since_day || day > window.until_day) {
            continue;
        }
        let model = input
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(crate::usage_pricing::strip_variant_suffix)
            .unwrap_or_else(|| "unknown".to_owned());
        let key = (
            day.format("%Y-%m-%d").to_string(),
            hour_start.clone().unwrap_or_default(),
            input.harness.clone(),
            model.clone(),
        );
        let bucket = buckets.entry(key).or_default();
        let mut priced = pricing.price(
            input.model.as_deref(),
            &input.tokens,
            input.reported_cost_microusd,
        );
        let mut cache_savings = pricing.cache_savings(input.model.as_deref(), &input.tokens);
        if input.cache_split_unknown && input.reported_cost_microusd.is_none() {
            priced = crate::usage_pricing::PricedUsage { cost_microusd: None, cost_source: CostSource::Unpriced };
            cache_savings = 0;
        }
        bucket.totals.add(&input.tokens);
        bucket.cost_microusd += priced.cost_microusd.unwrap_or(0);
        bucket.cache_savings_microusd += cache_savings;
        bucket.records += 1;
        match priced.cost_source {
            CostSource::Unpriced => bucket.unpriced_records += 1,
            CostSource::ProviderReported => bucket.provider_reported_records += 1,
            CostSource::ModelPriced => {}
        }
        if let Some(session) = input.session_id.filter(|session| !session.is_empty()) {
            bucket.sessions.insert(session);
        }
    }
    Ok(buckets
        .into_iter()
        .map(|((day, hour_start, harness, model), bucket)| UsageBucket {
            day,
            hour_start: (!hour_start.is_empty()).then_some(hour_start),
            harness,
            model,
            totals: bucket.totals,
            cost_microusd: bucket.cost_microusd,
            cache_savings_microusd: bucket.cache_savings_microusd,
            cost_source: resolve_cost_source(&bucket),
            records: bucket.records,
            unpriced_records: bucket.unpriced_records,
            sessions: Some(bucket.sessions.len() as i64),
        })
        .collect())
}

fn parse_instant(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|naive| Utc.from_utc_datetime(&naive))
        })
}

/// A live ledger row with the provider session its Bridge session bound to.
struct LiveRow {
    input: UsageInput,
    bridge_session_id: Option<String>,
    provider_session_id: Option<String>,
}

fn live_rows(
    db: &Connection,
    request: &UsageSummaryRequest,
    bounds: (DateTime<Utc>, DateTime<Utc>),
) -> Result<Vec<LiveRow>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT l.created_at,l.harness,l.model,l.serving_model,l.session_id,
                l.uncached_input_tokens,l.cache_read_tokens,l.cache_write_tokens,l.output_tokens,l.reasoning_tokens,
                l.cost_microusd,l.cost_source,s.provider_session_id,l.source
         FROM usage_ledger l LEFT JOIN sessions s ON s.id=l.session_id
         WHERE l.source LIKE 'provider.%'
           AND (?1 IS NULL OR l.workspace_id=?1)
           AND l.created_at >= ?2 AND l.created_at < ?3
         ORDER BY l.id",
    )?;
    let rows = statement.query_map(
        params![request.workspace_id, bounds.0.to_rfc3339(), bounds.1.to_rfc3339()],
        |row| {
            let created_at: String = row.get(0)?;
            let harness: Option<String> = row.get(1)?;
            let model: Option<String> = row.get(2)?;
            let serving_model: Option<String> = row.get(3)?;
            let session_id: Option<String> = row.get(4)?;
            let tokens = TokenUsage {
                uncached_input_tokens: row.get(5)?,
                cache_read_tokens: row.get(6)?,
                cache_write_tokens: row.get(7)?,
                output_tokens: row.get(8)?,
                reasoning_tokens: row.get(9)?,
                ..TokenUsage::default()
            };
            let cost: Option<i64> = row.get(10)?;
            let cost_source: Option<String> = row.get(11)?;
            let provider_session_id: Option<String> = row.get(12)?;
            let source: String = row.get(13)?;
            // Rows written before the session's harness was copied onto the
            // ledger still name their provider in `source`; that is the
            // harness, not an unknown.
            let harness = harness.or_else(|| source.strip_prefix("provider.").map(str::to_owned));
            Ok((created_at, harness, model, serving_model, session_id, tokens, cost, cost_source, provider_session_id))
        },
    )?;
    let mut live = Vec::new();
    for row in rows {
        let (created_at, harness, model, serving_model, session_id, tokens, cost, cost_source, provider_session_id) = row?;
        let Some(occurred_at) = parse_instant(&created_at) else { continue };
        let has_tokens = tokens.uncached_input_tokens.is_some()
            || tokens.cache_read_tokens.is_some()
            || tokens.cache_write_tokens.is_some()
            || tokens.output_tokens.is_some();
        let reported = (cost_source.as_deref() == Some(CostSource::ProviderReported.as_str()))
            .then_some(cost)
            .flatten();
        // A context gauge with neither tokens nor a cost is not a request.
        if !has_tokens && reported.is_none() {
            continue;
        }
        // Anthropic reports exclusive input, so a missing cache figure there is
        // a zero; every other provider reports cache-inclusive input, and the
        // adapter that wrote no cache figure at all did not know the split.
        let cache_split_unknown = !matches!(harness.as_deref(), Some("claude"))
            && tokens.uncached_input_tokens.is_some()
            && tokens.cache_read_tokens.is_none()
            && tokens.cache_write_tokens.is_none();
        live.push(LiveRow {
            input: UsageInput {
                occurred_at,
                harness: harness.unwrap_or_else(|| "unknown".into()),
                model: serving_model.or(model),
                session_id: session_id.clone(),
                tokens,
                reported_cost_microusd: reported,
                cache_split_unknown,
            },
            bridge_session_id: session_id,
            provider_session_id,
        });
    }
    Ok(live)
}

/// Imported observations with the native session they belong to.
fn imported_rows(
    db: &Connection,
    bounds: (DateTime<Utc>, DateTime<Utc>),
) -> Result<(Vec<UsageInput>, HashSet<String>), BridgeError> {
    let mut statement = db.prepare(
        "SELECT o.occurred_at,COALESCE(ses.agent,src.agent),o.model,ses.native_session_id,o.session_id,
                o.total_input_tokens,o.uncached_input_tokens,o.cache_read_tokens,o.cache_write_tokens,o.output_tokens,o.reasoning_tokens,
                o.exact_total_formula,o.reported_cost_microusd
         FROM agent_usage_observations o
         LEFT JOIN agent_usage_sessions ses ON ses.id=o.session_id
         LEFT JOIN agent_usage_sources src ON src.id=o.source_id
         WHERE src.coverage_state='complete'
           AND o.occurred_at >= ?1 AND o.occurred_at < ?2
         ORDER BY o.occurred_at, o.id",
    )?;
    let rows = statement.query_map(params![bounds.0.to_rfc3339(), bounds.1.to_rfc3339()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, Option<i64>>(9)?,
            row.get::<_, Option<i64>>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, Option<i64>>(12)?,
        ))
    })?;
    let mut inputs = Vec::new();
    let mut native_sessions = HashSet::new();
    for row in rows {
        let (occurred_at, agent, model, native_session_id, session_row_id, total_input, uncached, cache_read, cache_write, output, reasoning, formula, reported) = row?;
        let Some(occurred_at) = parse_instant(&occurred_at) else { continue };
        // Only the cache-inclusive formula lets an exclusive figure be derived;
        // anything else stays as the importer wrote it.
        let uncached = uncached.or_else(|| {
            (formula == ExactTotalFormula::InputIncludesCachePlusOutput.as_str())
                .then(|| {
                    total_input.map(|total| {
                        total
                            .saturating_sub(cache_read.unwrap_or(0))
                            .saturating_sub(cache_write.unwrap_or(0))
                            .max(0)
                    })
                })
                .flatten()
        });
        if let Some(native) = &native_session_id {
            native_sessions.insert(native.clone());
        }
        inputs.push(UsageInput {
            occurred_at,
            harness: agent.unwrap_or_else(|| "unknown".into()),
            model,
            session_id: native_session_id.or(session_row_id),
            tokens: TokenUsage {
                total_input_tokens: total_input,
                uncached_input_tokens: uncached,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                output_tokens: output,
                reasoning_tokens: reasoning,
                ..TokenUsage::default()
            },
            reported_cost_microusd: reported,
            cache_split_unknown: false,
        });
    }
    Ok((inputs, native_sessions))
}

fn sources(db: &Connection) -> Result<Vec<UsageSummarySource>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id,agent,provider,coverage_state,coverage_reason,records_imported,records_skipped,last_successful_scan_at
         FROM agent_usage_sources ORDER BY agent, id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(UsageSummarySource {
            id: row.get(0)?,
            origin: Default::default(),
            agent: row.get(1)?,
            provider: row.get(2)?,
            coverage_state: row.get(3)?,
            coverage_reason: row.get(4)?,
            records_imported: row.get(5)?,
            records_skipped: row.get(6)?,
            last_successful_scan_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The `usage/summary` body.
pub fn summarize(db: &Connection, request: &UsageSummaryRequest) -> Result<UsageSummary, BridgeError> {
    summarize_excluding(db, request, None)
}

/// An account dashboard is an exclusive authority for its harness. Select it
/// before aggregation so its local rows cannot inflate counts or totals.
pub(crate) fn summarize_excluding(
    db: &Connection,
    request: &UsageSummaryRequest,
    excluded_harness: Option<&str>,
) -> Result<UsageSummary, BridgeError> {
    let started = Instant::now();
    let window = Window::parse(request)?;
    let bounds = window.coarse_bounds();
    let pricing = Pricing::load(db)?;

    let live = live_rows(db, request, bounds)?;
    let (mut imported, native_sessions) = if request.include_imported {
        imported_rows(db, bounds)?
    } else {
        (Vec::new(), HashSet::new())
    };

    imported.retain(|row| Some(row.harness.as_str()) != excluded_harness);
    let mut duplicates_dropped = 0_i64;
    let mut inputs: Vec<UsageInput> = Vec::with_capacity(live.len() + imported.len());
    for row in live {
        if Some(row.input.harness.as_str()) == excluded_harness {
            continue;
        }
        let covered = row
            .provider_session_id
            .as_deref()
            .is_some_and(|provider_session| native_sessions.contains(provider_session));
        if covered {
            duplicates_dropped += 1;
            continue;
        }
        let _ = row.bridge_session_id;
        inputs.push(row.input);
    }
    let live_records = inputs.len() as i64;
    let imported_records = imported.len() as i64;
    inputs.extend(imported);

    let buckets = aggregate(request, &pricing, inputs)?;
    Ok(UsageSummary {
        since_day: window.since_day.format("%Y-%m-%d").to_string(),
        until_day: window.until_day.format("%Y-%m-%d").to_string(),
        time_zone: window.zone.name().to_owned(),
        resolution: request.resolution,
        buckets,
        sources: sources(db)?,
        pricing: pricing.status().clone(),
        scan_duration_ms: started.elapsed().as_millis() as i64,
        duplicates_dropped,
        live_records,
        imported_records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn request(since: &str, until: &str, zone: Option<&str>) -> UsageSummaryRequest {
        UsageSummaryRequest {
            since_day: since.into(),
            until_day: until.into(),
            resolution: UsageResolution::Day,
            time_zone: zone.map(str::to_owned),
            workspace_id: None,
            include_imported: false,
            include_dashboard: false,
            since_time: None,
            until_time: None,
        }
    }

    fn input(at: &str, harness: &str, model: &str, session: &str, output: i64, cost: Option<i64>) -> UsageInput {
        UsageInput {
            occurred_at: DateTime::parse_from_rfc3339(at).unwrap().with_timezone(&Utc),
            harness: harness.into(),
            model: Some(model.into()),
            session_id: Some(session.into()),
            cache_split_unknown: false,
            tokens: TokenUsage {
                uncached_input_tokens: Some(1_000),
                cache_read_tokens: Some(0),
                cache_write_tokens: Some(0),
                output_tokens: Some(output),
                reasoning_tokens: Some(output / 2),
                ..TokenUsage::default()
            },
            reported_cost_microusd: cost,
        }
    }

    #[test]
    fn day_buckets_follow_the_callers_time_zone() {
        let pricing = Pricing::bundled_only();
        // 04:30Z on the 2nd is 23:30 on the 1st in New York (UTC-5 in January).
        let record = input("2026-01-02T04:30:00Z", "codex", "gpt-5", "s1", 10, None);
        let local = aggregate(&request("2026-01-01", "2026-01-01", Some("America/New_York")), &pricing, [record.clone()]).unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].day, "2026-01-01");
        let utc = aggregate(&request("2026-01-01", "2026-01-01", None), &pricing, [record]).unwrap();
        assert!(utc.is_empty(), "in UTC the same record belongs to the 2nd");
        assert!(aggregate(&request("2026-01-01", "2026-01-01", Some("Mars/Olympus")), &pricing, []).is_err());
    }

    #[test]
    fn hour_resolution_requires_exact_bounds_within_a_day() {
        let pricing = Pricing::bundled_only();
        let mut hourly = request("2026-01-01", "2026-01-01", None);
        hourly.resolution = UsageResolution::Hour;
        assert!(aggregate(&hourly, &pricing, []).is_err(), "no bounds");
        hourly.since_time = Some("2026-01-01T00:00:00Z".into());
        hourly.until_time = Some("2026-01-02T00:00:01Z".into());
        assert!(aggregate(&hourly, &pricing, []).is_err(), "over 24 h");
        hourly.until_time = Some("2026-01-01T06:00:00Z".into());
        let buckets = aggregate(
            &hourly,
            &pricing,
            [
                input("2026-01-01T01:10:00Z", "codex", "gpt-5", "s1", 10, None),
                input("2026-01-01T01:50:00Z", "codex", "gpt-5", "s1", 10, None),
                input("2026-01-01T02:05:00Z", "codex", "gpt-5", "s1", 10, None),
                input("2026-01-01T06:00:00Z", "codex", "gpt-5", "s1", 10, None),
            ],
        )
        .unwrap();
        assert_eq!(buckets.len(), 2, "the record at the exclusive end is out");
        assert_eq!(buckets[0].hour_start.as_deref(), Some("2026-01-01T01:00:00Z"));
        assert_eq!(buckets[0].records, 2);
        assert_eq!(buckets[1].hour_start.as_deref(), Some("2026-01-01T02:00:00Z"));
    }

    #[test]
    fn the_weakest_provenance_in_a_bucket_wins() {
        let pricing = Pricing::bundled_only();
        let window = request("2026-01-01", "2026-01-01", None);
        let reported = aggregate(&window, &pricing, [
            input("2026-01-01T01:00:00Z", "claude", "claude-opus-4-6", "s1", 10, Some(500)),
            input("2026-01-01T02:00:00Z", "claude", "claude-opus-4-6", "s1", 10, Some(700)),
        ]).unwrap();
        assert_eq!(reported[0].cost_source, CostSource::ProviderReported);
        assert_eq!(reported[0].cost_microusd, 1_200);

        let mixed = aggregate(&window, &pricing, [
            input("2026-01-01T01:00:00Z", "claude", "claude-opus-4-6", "s1", 10, Some(500)),
            input("2026-01-01T02:00:00Z", "claude", "claude-opus-4-6", "s1", 10, None),
        ]).unwrap();
        assert_eq!(mixed[0].cost_source, CostSource::ModelPriced);
        assert!(mixed[0].cost_microusd > 500);
        assert_eq!(mixed[0].unpriced_records, 0);

        let unpriced = aggregate(&window, &pricing, [
            input("2026-01-01T01:00:00Z", "codex", "mystery", "s1", 10, None),
            input("2026-01-01T02:00:00Z", "codex", "mystery", "s2", 10, None),
        ]).unwrap();
        assert_eq!(unpriced[0].cost_source, CostSource::Unpriced);
        assert_eq!(unpriced[0].cost_microusd, 0);
        assert_eq!(unpriced[0].unpriced_records, 2);
        assert_eq!(unpriced[0].totals.output_tokens, 20, "tokens still count");
        assert_eq!(unpriced[0].totals.reasoning_tokens, 10);
        assert_eq!(unpriced[0].sessions, Some(2));

        // Only unpriced when nothing priced: one priced row lifts the bucket.
        let partly = aggregate(&window, &pricing, [
            input("2026-01-01T01:00:00Z", "codex", "gpt-5", "s1", 10, None),
            input("2026-01-01T02:00:00Z", "codex", "gpt-5", "s1", 10, Some(1)),
        ]).unwrap();
        assert_eq!(partly[0].cost_source, CostSource::ModelPriced);
    }

    #[test]
    fn a_context_variant_buckets_with_its_base_model() {
        let pricing = Pricing::bundled_only();
        let window = request("2026-01-01", "2026-01-01", None);
        let buckets = aggregate(&window, &pricing, [
            input("2026-01-01T01:00:00Z", "claude", "claude-opus-5-5[1m]", "s1", 10, None),
            input("2026-01-01T02:00:00Z", "claude", "claude-opus-5-5", "s2", 10, None),
        ]).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].model, "claude-opus-5-5");
        assert_eq!(buckets[0].records, 2);
        assert_eq!(buckets[0].cost_source, CostSource::ModelPriced);
    }

    #[test]
    fn buckets_sort_by_day_hour_harness_and_model() {
        let pricing = Pricing::bundled_only();
        let window = request("2026-01-01", "2026-01-03", None);
        let records = [
            input("2026-01-02T01:00:00Z", "codex", "gpt-5", "s1", 1, None),
            input("2026-01-01T01:00:00Z", "opencode", "anthropic/claude-opus-4-6", "s2", 1, None),
            input("2026-01-01T01:00:00Z", "claude", "claude-opus-4-6", "s3", 1, None),
            input("2026-01-01T01:00:00Z", "claude", "claude-haiku-4-5", "s3", 1, None),
        ];
        let first = aggregate(&window, &pricing, records.clone()).unwrap();
        let keys: Vec<(String, String, String)> = first.iter().map(|b| (b.day.clone(), b.harness.clone(), b.model.clone())).collect();
        assert_eq!(keys, vec![
            ("2026-01-01".into(), "claude".into(), "claude-haiku-4-5".into()),
            ("2026-01-01".into(), "claude".into(), "claude-opus-4-6".into()),
            ("2026-01-01".into(), "opencode".into(), "anthropic/claude-opus-4-6".into()),
            ("2026-01-02".into(), "codex".into(), "gpt-5".into()),
        ]);
        let mut reversed = records.to_vec();
        reversed.reverse();
        assert_eq!(aggregate(&window, &pricing, reversed).unwrap(), first, "order of input is irrelevant");
    }

    fn seeded() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/usage-demo','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/usage-demo/w','ready','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,model,provider_session_id) VALUES('live-covered','w','claude','a','ready','claude-opus-4-6','native-1');
             INSERT INTO sessions(id,workspace_id,harness,label,status,model,provider_session_id) VALUES('live-alone','w','claude','b','ready','claude-opus-4-6','native-2');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,cost_microusd,cost_source,harness,model,source,created_at)
                 VALUES('w','live-covered','t1',100,0,0,10,5000,'provider_reported','claude','claude-opus-4-6','provider.claude','2026-01-01T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,cost_microusd,cost_source,harness,model,source,created_at)
                 VALUES('w','live-alone','t2',100,0,0,10,7000,'provider_reported','claude','claude-opus-4-6','provider.claude','2026-01-01T11:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,context_percent,harness,model,source,created_at)
                 VALUES('w','live-alone','t3',40,'cursor',NULL,'provider.cursor','2026-01-01T11:30:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,capability_units,source,created_at)
                 VALUES('w','live-alone','t2',2,'policy.spawn.strong','2026-01-01T11:00:00+00:00');
             INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,records_imported,importer_version,created_at,updated_at)
                 VALUES('src','claude','anthropic','fp','complete',2,'test','now','now');
             INSERT INTO agent_usage_sessions(id,source_id,native_session_id,agent,provider,created_at,updated_at) VALUES('ases','src','native-1','claude','anthropic','now','now');
             INSERT INTO agent_usage_observations(id,source_id,session_id,native_record_id,occurred_at,model,input_semantics,output_semantics,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,exact_total_formula,reported_cost_microusd,importer_version,created_at)
                 VALUES('o1','src','ases','r1','2026-01-01T10:00:00+00:00','claude-opus-4-6','exclusive','delta',100,50,0,10,'anthropic_exclusive_input_plus_cache_and_output',5000,'test','now');
             INSERT INTO agent_usage_observations(id,source_id,session_id,native_record_id,occurred_at,model,input_semantics,output_semantics,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,exact_total_formula,reported_cost_microusd,importer_version,created_at)
                 VALUES('o2','src','ases','r2','2026-01-01T10:05:00+00:00','claude-opus-4-6','exclusive','delta',100,50,0,10,'anthropic_exclusive_input_plus_cache_and_output',6000,'test','now');",
        )
        .unwrap();
        db
    }

    #[test]
    fn a_live_row_without_a_harness_is_attributed_to_its_source_provider() {
        let db = seeded();
        db.execute_batch(
            "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,uncached_input_tokens,cache_read_tokens,output_tokens,harness,model,source,created_at)
                 VALUES('w','live-alone','t9',10,0,5,NULL,NULL,'provider.codex','2026-01-01T12:00:00+00:00');",
        )
        .unwrap();
        let summary = summarize(&db, &request("2026-01-01", "2026-01-01", None)).unwrap();
        let harnesses: Vec<&str> = summary.buckets.iter().map(|bucket| bucket.harness.as_str()).collect();
        assert!(harnesses.contains(&"codex"), "{harnesses:?}");
        assert!(!harnesses.contains(&"unknown"), "{harnesses:?}");
    }

    #[test]
    fn a_cache_inclusive_row_with_no_cache_figure_counts_tokens_but_is_not_priced() {
        let db = seeded();
        db.execute_batch(
            "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,harness,model,source,created_at)
                 VALUES('w','live-alone','t8',400000,400000,500,'codex','gpt-5',  'provider.codex','2026-01-01T12:00:00+00:00');",
        )
        .unwrap();
        let summary = summarize(&db, &request("2026-01-01", "2026-01-01", None)).unwrap();
        let codex = summary.buckets.iter().find(|bucket| bucket.harness == "codex").expect("a codex bucket");
        assert_eq!(codex.totals.uncached_input_tokens, 400_000, "the tokens the provider named are counted");
        assert_eq!(codex.cost_microusd, 0, "an unknown cache split is not priced at the full rate");
        assert_eq!(codex.cache_savings_microusd, 0);
        assert_eq!(codex.unpriced_records, 1);
        assert_eq!(codex.cost_source, CostSource::Unpriced);
    }

    #[test]
    fn a_session_covered_by_an_import_contributes_only_its_observations() {
        let db = seeded();
        let mut request = request("2026-01-01", "2026-01-01", None);
        let live_only = summarize(&db, &request).unwrap();
        assert_eq!(live_only.duplicates_dropped, 0);
        assert_eq!(live_only.live_records, 2, "the policy row and the bare gauge are not requests");
        assert_eq!(live_only.imported_records, 0);
        assert_eq!(live_only.buckets.len(), 1);
        assert_eq!(live_only.buckets[0].cost_microusd, 12_000);
        assert_eq!(live_only.buckets[0].sessions, Some(2));
        assert_eq!(live_only.pricing.status, "bundled");
        assert_eq!(live_only.sources.len(), 1);

        request.include_imported = true;
        db.execute("UPDATE agent_usage_sources SET coverage_state='partial' WHERE id='src'", [])
            .unwrap();
        let rebuilding = summarize(&db, &request).unwrap();
        assert_eq!(rebuilding.duplicates_dropped, 0);
        assert_eq!(rebuilding.live_records, 2);
        assert_eq!(rebuilding.imported_records, 0);
        assert_eq!(rebuilding.buckets[0].cost_microusd, 12_000);

        db.execute("UPDATE agent_usage_sources SET coverage_state='complete' WHERE id='src'", [])
            .unwrap();
        let merged = summarize(&db, &request).unwrap();
        assert_eq!(merged.duplicates_dropped, 1, "live-covered's row is set aside");
        assert_eq!(merged.live_records, 1);
        assert_eq!(merged.imported_records, 2);
        assert_eq!(merged.buckets.len(), 1);
        let bucket = &merged.buckets[0];
        assert_eq!(bucket.records, 3);
        assert_eq!(bucket.cost_microusd, 7_000 + 5_000 + 6_000);
        assert_eq!(bucket.cost_source, CostSource::ProviderReported);
        assert_eq!(bucket.totals.cache_read_tokens, 100);
        assert_eq!(bucket.sessions, Some(2), "live-alone plus native-1");

        request.workspace_id = Some("elsewhere".into());
        let scoped = summarize(&db, &request).unwrap();
        assert_eq!(scoped.live_records, 0);
        assert_eq!(scoped.imported_records, 2, "imports are device-wide, not per workspace");
    }
}
