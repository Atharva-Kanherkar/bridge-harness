//! Token and cost usage: day and hour roll-ups of the usage ledger, user
//! price overrides, and the explicit rate-table refresh.
//!
//! Every figure is an integer. Rates are micro-USD per million tokens, costs
//! are micro-USD, and a bucket with no rate is `unpriced` with zero cost while
//! its tokens still count. Nothing here reaches the network except
//! `usage/refresh_rates`, which a client must ask for by name.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mirrors `bridge_core::usage_summary::UsageResolution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageResolution {
    Day,
    Hour,
}

/// Why a cost is what it is. Mirrors `bridge_core::usage_pricing::CostSource`.
///
/// - `provider_reported` — every row carried the provider's own figure.
/// - `model_priced` — a user override or the rate table priced at least one row.
/// - `unpriced` — tokens are known, rates are not; counted, not costed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageCostSource {
    ProviderReported,
    ModelPriced,
    Unpriced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SummaryParams {
    /// Inclusive first day, `YYYY-MM-DD` in `timeZone`.
    pub since_day: String,
    /// Inclusive last day, `YYYY-MM-DD` in `timeZone`.
    pub until_day: String,
    pub resolution: UsageResolution,
    /// IANA zone to bucket days in; UTC when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// Restrict live rows to one workspace. Imported observations are
    /// device-wide and are never scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Merge observations imported from provider transcripts, de-duplicated
    /// against live rows by provider session.
    pub include_imported: bool,
    /// Inclusive UTC start, RFC 3339. Required for `hour` resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_time: Option<String>,
    /// Exclusive UTC end, RFC 3339, at most 24 hours after `sinceTime`.
    /// Required for `hour` resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_time: Option<String>,
}

/// Mirrors `bridge_core::usage_summary::UsageBucketTotals`. Mutually
/// exclusive buckets; `reasoningTokens` is a breakdown of `outputTokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucketTotals {
    pub uncached_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
}

/// One `(day, hourStart?, harness, model)` cell. Mirrors
/// `bridge_core::usage_summary::UsageBucket`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub day: String,
    /// UTC start of the hour, present only for hour resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour_start: Option<String>,
    pub harness: String,
    pub model: String,
    pub totals: UsageBucketTotals,
    pub cost_microusd: i64,
    pub cache_savings_microusd: i64,
    pub cost_source: UsageCostSource,
    pub records: i64,
    pub unpriced_records: i64,
    pub sessions: i64,
}

/// A history importer's standing. Mirrors
/// `bridge_core::usage_summary::UsageSummarySource`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummarySource {
    pub id: String,
    pub agent: String,
    pub provider: String,
    pub coverage_state: String,
    pub coverage_reason: Option<String>,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub last_successful_scan_at: Option<String>,
}

/// Where the rate table came from. Mirrors
/// `bridge_core::usage_pricing::PricingStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsagePricingStatus {
    /// `bundled`, `fresh`, `cached`, or `unavailable`.
    pub status: String,
    pub fetched_at: Option<String>,
    pub snapshot_date: String,
    pub source: String,
    pub known_models: i64,
    pub overrides: i64,
}

/// Mirrors `bridge_core::usage_summary::UsageSummary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummaryResult {
    pub since_day: String,
    pub until_day: String,
    pub time_zone: String,
    pub resolution: UsageResolution,
    pub buckets: Vec<UsageBucket>,
    pub sources: Vec<UsageSummarySource>,
    pub pricing: UsagePricingStatus,
    pub scan_duration_ms: i64,
    pub duplicates_dropped: i64,
    pub live_records: i64,
    pub imported_records: i64,
}

/// A user's rate for one model, micro-USD per million tokens. Mirrors
/// `bridge_core::usage_pricing::PriceOverride`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsagePriceOverride {
    pub model: String,
    pub input_microusd_per_mtok: i64,
    pub output_microusd_per_mtok: i64,
    pub cache_read_microusd_per_mtok: Option<i64>,
    pub cache_write_microusd_per_mtok: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListUsagePriceOverridesResult(pub Vec<UsagePriceOverride>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetPriceOverrideParams {
    pub model: String,
    pub input_microusd_per_mtok: i64,
    pub output_microusd_per_mtok: i64,
    /// Defaults to the input rate when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_microusd_per_mtok: Option<i64>,
    /// Defaults to the input rate when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_microusd_per_mtok: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClearPriceOverrideParams {
    pub model: String,
}

/// Mirrors `bridge_core::analytics::ImporterCapability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageImporterCapability {
    Supported,
    Unsupported,
}

/// Mirrors `bridge_core::analytics::CoverageState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageCoverageState {
    Complete,
    Partial,
    Stale,
    Unsupported,
    Unreadable,
    Empty,
}

/// A history source the importers know how to look for, with what has been
/// indexed from it. Mirrors `bridge_core::usage_history::UsageHistorySource`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistorySource {
    pub id: String,
    pub agent: String,
    pub provider: String,
    pub location: String,
    pub detected_version: Option<String>,
    pub capability: UsageImporterCapability,
    pub coverage_state: UsageCoverageState,
    pub coverage_reason: Option<String>,
    pub coverage_start_at: Option<String>,
    pub coverage_end_at: Option<String>,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub last_successful_scan_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListHistorySourcesResult(pub Vec<UsageHistorySource>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanHistoryParams {
    /// Records to import per source this call, clamped to 1..=10000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_records: Option<u64>,
    /// Only these sources; every id must be one `list_history_sources`
    /// returned. All discovered sources when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ids: Option<Vec<String>>,
}

/// Mirrors `bridge_core::usage_import::SourceScanOutcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryScanOutcome {
    pub source_id: String,
    pub agent: String,
    pub provider: String,
    pub location: String,
    pub capability: UsageImporterCapability,
    pub coverage: UsageCoverageState,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub next_cursor: Option<String>,
    pub warning: Option<String>,
}

/// Mirrors `bridge_core::usage_import::ScanReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScanHistoryResult {
    pub sources: Vec<UsageHistoryScanOutcome>,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub duration_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn usage_params_refuse_unknown_fields_and_spell_enums_on_the_wire() {
        let params: SummaryParams = serde_json::from_value(json!({
            "sinceDay": "2026-01-01", "untilDay": "2026-01-07", "resolution": "day", "includeImported": true
        }))
        .unwrap();
        assert_eq!(params.resolution, UsageResolution::Day);
        assert_eq!(round_trip(&params), params);
        assert!(serde_json::from_value::<SummaryParams>(json!({
            "sinceDay": "2026-01-01", "untilDay": "2026-01-07", "resolution": "day", "includeImported": true, "extra": 1
        }))
        .is_err());
        assert_eq!(serde_json::to_value(UsageCostSource::ProviderReported).unwrap(), json!("provider_reported"));
        assert_eq!(serde_json::to_value(UsageResolution::Hour).unwrap(), json!("hour"));
    }

    #[test]
    fn a_day_bucket_omits_hour_start() {
        let bucket = UsageBucket {
            day: "2026-01-01".into(),
            hour_start: None,
            harness: "codex".into(),
            model: "gpt-5".into(),
            totals: UsageBucketTotals {
                uncached_input_tokens: 1,
                cache_read_tokens: 2,
                cache_write_tokens: 3,
                output_tokens: 4,
                reasoning_tokens: 1,
            },
            cost_microusd: 5,
            cache_savings_microusd: 1,
            cost_source: UsageCostSource::ModelPriced,
            records: 1,
            unpriced_records: 0,
            sessions: 1,
        };
        let wire = serde_json::to_value(&bucket).unwrap();
        assert!(wire.get("hourStart").is_none());
        assert_eq!(wire["costSource"], json!("model_priced"));
        assert_eq!(round_trip(&bucket), bucket);
    }
}
