//! Model rate lookup and cost arithmetic for the usage ledger.
//!
//! Rates come from LiteLLM's `model_prices_and_context_window.json`, projected
//! to the handful of fields Bridge prices against and bundled into the binary
//! as `usage_rates.json`. A fresher copy is fetched only by the explicit
//! `usage/refresh_rates` method and cached in `usage_rate_cache`; nothing here
//! touches the network on its own.
//!
//! Every figure is an integer: rates are micro-USD per million tokens, costs
//! are micro-USD, and rounding is half-up on the final sum. Base tier only —
//! LiteLLM's long-context and service tiers are not known per request, so a
//! `[1m]` variant suffix is stripped and the base rate used.

use crate::analytics::TokenUsage;
use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const BUNDLED_RATES_JSON: &str = include_str!("usage_rates.json");
pub const RATE_SOURCE_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

const MICRO_PER_MILLION: i128 = 1_000_000;

/// Micro-USD per million tokens for one model. Cache rates fall back to the
/// input rate when a provider does not publish them: cached input priced as
/// plain input is a conservative figure, cached input priced as free is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRate {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

/// The snapshot document, as bundled and as cached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateDocument {
    snapshot_date: String,
    source: String,
    #[serde(default)]
    unit: String,
    rates: BTreeMap<String, RawRate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRate {
    input: i64,
    output: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_read: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_write: Option<i64>,
}

/// Parsed rates plus the bare-name aliases that resolve unambiguously.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateTable {
    rates: BTreeMap<String, ModelRate>,
    aliases: BTreeMap<String, ModelRate>,
}

/// A rate table with the provenance of the document it was built from.
#[derive(Debug, Clone)]
pub struct RateSnapshot {
    pub table: RateTable,
    pub snapshot_date: String,
    pub source: String,
}

/// Models never priced, regardless of the table. `<synthetic>` marks locally
/// generated messages that were never billed; bare family names are ambiguous
/// across generations, so they report as unpriced rather than guess one.
const UNPRICEABLE_MODELS: &[&str] = &["<synthetic>", "synthetic", "opus", "sonnet", "haiku", "fable"];

impl RateTable {
    fn from_raw(rates: &BTreeMap<String, RawRate>) -> Self {
        let mut table = BTreeMap::new();
        for (name, raw) in rates {
            let key = normalize_rate_key(name);
            if key.is_empty() {
                continue;
            }
            table.insert(
                key,
                ModelRate {
                    input: raw.input,
                    output: raw.output,
                    cache_read: raw.cache_read.unwrap_or(raw.input),
                    cache_write: raw.cache_write.unwrap_or(raw.input),
                },
            );
        }
        // A bare name is an alias only when no canonical entry exists for it
        // and every qualified entry agrees on the rate. `None` marks a bare
        // name claimed at conflicting rates.
        let mut candidates: BTreeMap<String, Option<ModelRate>> = BTreeMap::new();
        for (key, rate) in &table {
            let bare = bare_model_name(key);
            if bare.is_empty() || bare == key || table.contains_key(bare) {
                continue;
            }
            match candidates.get(bare) {
                None => {
                    candidates.insert(bare.to_owned(), Some(*rate));
                }
                Some(Some(held)) if held != rate => {
                    candidates.insert(bare.to_owned(), None);
                }
                _ => {}
            }
        }
        let aliases = candidates
            .into_iter()
            .filter_map(|(alias, rate)| rate.map(|rate| (alias, rate)))
            .collect();
        Self { rates: table, aliases }
    }

    pub fn len(&self) -> usize {
        self.rates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }

    pub fn rates(&self) -> impl Iterator<Item = (&str, &ModelRate)> {
        self.rates.iter().map(|(key, rate)| (key.as_str(), rate))
    }

    /// The base-tier rate for a model id as a harness reports it.
    ///
    /// Lowercases, strips a `[1m]`-style variant suffix, and resolves a
    /// provider-prefixed id to its bare name only when that is unambiguous:
    /// either a canonical bare entry exists, or every qualified entry sharing
    /// the bare name agrees on the rate. Bare family names never resolve.
    pub fn lookup_rate(&self, model: &str) -> Option<ModelRate> {
        let key = strip_variant_suffix(&normalize_rate_key(model));
        let bare = bare_model_name(&key);
        if bare.is_empty() || UNPRICEABLE_MODELS.contains(&bare) {
            return None;
        }
        self.rates
            .get(&key)
            .or_else(|| self.aliases.get(&key))
            .or_else(|| self.rates.get(bare))
            .or_else(|| self.aliases.get(bare))
            .copied()
    }
}

fn normalize_rate_key(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn bare_model_name(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Drops a bracketed variant suffix such as `claude-opus-4-6[1m]`, which
/// Claude Code writes for the 1M context tier.
pub(crate) fn strip_variant_suffix(key: &str) -> String {
    match key.find('[') {
        Some(index) => key[..index].to_owned(),
        None => key.to_owned(),
    }
}

fn parse_snapshot(json: &str) -> Result<RateSnapshot, BridgeError> {
    let document: RateDocument = serde_json::from_str(json)
        .map_err(|error| BridgeError::Invalid(format!("usage rate table is unreadable: {error}")))?;
    Ok(RateSnapshot {
        table: RateTable::from_raw(&document.rates),
        snapshot_date: document.snapshot_date,
        source: document.source,
    })
}

/// The rate table compiled into the binary.
pub fn bundled() -> &'static RateSnapshot {
    static BUNDLED: OnceLock<RateSnapshot> = OnceLock::new();
    BUNDLED.get_or_init(|| parse_snapshot(BUNDLED_RATES_JSON).expect("the bundled rate table parses"))
}

/// LiteLLM providers whose entries name models a Claude Code, Codex, OpenCode
/// or Cursor session can plausibly run. Everything else — regional Bedrock
/// duplicates, resellers, image and audio models — is left out so the table
/// stays small and unambiguous.
const KEPT_PROVIDERS: &[&str] = &[
    "anthropic", "openai", "gemini", "xai", "moonshot", "deepseek", "zai", "dashscope", "minimax",
];
const KEPT_PREFIXES: &[&str] = &[
    "claude-", "gpt-", "chatgpt-", "o1", "o3", "o4", "codex", "gemini-", "grok-", "kimi-",
    "deepseek-", "glm-", "qwen", "minimax",
];
const DROPPED_FRAGMENTS: &[&str] = &[
    "ft:", "realtime", "audio", "search", "tts", "transcribe", "image", "embedding", "moderation",
    "computer-use", "live", "native", "batch", "preview-",
];

/// USD per token → micro-USD per million tokens, rounded half up.
fn usd_per_token_to_micro_per_million(value: f64) -> Option<i64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * 1e12).round() as i64)
}

/// Projects a raw LiteLLM document to the snapshot shape the bundled file
/// uses: base-tier input/output/cache rates for the models Bridge can meet.
pub fn project_litellm_document(document: &Value, snapshot_date: &str) -> String {
    let mut rates: BTreeMap<String, RawRate> = BTreeMap::new();
    let Some(entries) = document.as_object() else {
        return serialize_document(snapshot_date, &rates);
    };
    for (name, entry) in entries {
        let Some(entry) = entry.as_object() else { continue };
        let provider = entry.get("litellm_provider").and_then(Value::as_str).unwrap_or("");
        if !KEPT_PROVIDERS.contains(&provider) {
            continue;
        }
        let mode = entry.get("mode").and_then(Value::as_str).unwrap_or("");
        if !matches!(mode, "chat" | "responses") {
            continue;
        }
        let (Some(input), Some(output)) = (
            entry.get("input_cost_per_token").and_then(Value::as_f64),
            entry.get("output_cost_per_token").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let key = normalize_rate_key(name);
        if DROPPED_FRAGMENTS.iter().any(|fragment| key.contains(fragment)) {
            continue;
        }
        let bare = bare_model_name(&key);
        if !KEPT_PREFIXES.iter().any(|prefix| bare.starts_with(prefix)) {
            continue;
        }
        let (Some(input), Some(output)) = (
            usd_per_token_to_micro_per_million(input),
            usd_per_token_to_micro_per_million(output),
        ) else {
            continue;
        };
        // A free experimental tier is not a price; leaving it out keeps the
        // model honestly unpriced instead of costing nothing.
        if input == 0 && output == 0 {
            continue;
        }
        rates.insert(
            key,
            RawRate {
                input,
                output,
                cache_read: entry
                    .get("cache_read_input_token_cost")
                    .and_then(Value::as_f64)
                    .and_then(usd_per_token_to_micro_per_million),
                cache_write: entry
                    .get("cache_creation_input_token_cost")
                    .and_then(Value::as_f64)
                    .and_then(usd_per_token_to_micro_per_million),
            },
        );
    }
    serialize_document(snapshot_date, &rates)
}

fn serialize_document(snapshot_date: &str, rates: &BTreeMap<String, RawRate>) -> String {
    serde_json::to_string(&RateDocument {
        snapshot_date: snapshot_date.into(),
        source: RATE_SOURCE_URL.into(),
        unit: "microusd_per_million_tokens".into(),
        rates: rates.clone(),
    })
    .expect("rate documents serialize")
}

/// Why a cost is what it is. The wire spelling is snake_case, matching the
/// `cost_source` column the ledger already carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    ProviderReported,
    ModelPriced,
    Unpriced,
}

impl CostSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderReported => "provider_reported",
            Self::ModelPriced => "model_priced",
            Self::Unpriced => "unpriced",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "provider_reported" => Some(Self::ProviderReported),
            "model_priced" => Some(Self::ModelPriced),
            "unpriced" => Some(Self::Unpriced),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PricedUsage {
    /// `None` only when unpriced: tokens are known, rates are not.
    pub cost_microusd: Option<i64>,
    pub cost_source: CostSource,
}

/// A user's own rate for a model, in micro-USD per million tokens. The key
/// keeps its case, prefix, and suffix exactly as entered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceOverride {
    pub model: String,
    pub input_microusd_per_mtok: i64,
    pub output_microusd_per_mtok: i64,
    pub cache_read_microusd_per_mtok: Option<i64>,
    pub cache_write_microusd_per_mtok: Option<i64>,
    pub updated_at: String,
}

impl PriceOverride {
    fn rate(&self) -> ModelRate {
        ModelRate {
            input: self.input_microusd_per_mtok,
            output: self.output_microusd_per_mtok,
            cache_read: self.cache_read_microusd_per_mtok.unwrap_or(self.input_microusd_per_mtok),
            cache_write: self
                .cache_write_microusd_per_mtok
                .unwrap_or(self.input_microusd_per_mtok),
        }
    }
}

/// Provenance for the rate table, so a reader can judge the cost figures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingStatus {
    /// `bundled`, `fresh`, `cached`, or `unavailable`.
    pub status: String,
    pub fetched_at: Option<String>,
    pub snapshot_date: String,
    pub source: String,
    pub known_models: i64,
    pub overrides: i64,
}

/// Everything needed to price a row: the rate table in force, the user's
/// overrides, and where the table came from.
#[derive(Debug, Clone)]
pub struct Pricing {
    table: RateTable,
    /// The bundled snapshot, consulted when a refreshed table lacks a model
    /// Bridge already knows how to price (a model newer than the refresh).
    fallback: Option<RateTable>,
    overrides: BTreeMap<String, ModelRate>,
    status: PricingStatus,
}

impl Pricing {
    /// The bundled table with no overrides — for callers without a database.
    pub fn bundled_only() -> Self {
        let snapshot = bundled();
        Self {
            table: snapshot.table.clone(),
            fallback: None,
            overrides: BTreeMap::new(),
            status: PricingStatus {
                status: "bundled".into(),
                fetched_at: None,
                snapshot_date: snapshot.snapshot_date.clone(),
                source: snapshot.source.clone(),
                known_models: snapshot.table.len() as i64,
                overrides: 0,
            },
        }
    }

    /// The table in force for this data directory: the explicitly refreshed
    /// cache when one exists and parses, backed by the bundled snapshot for
    /// models the cache predates; otherwise the bundled snapshot alone.
    pub fn load(db: &Connection) -> Result<Self, BridgeError> {
        let mut pricing = Self::bundled_only();
        if let Some(cached) = rate_cache(db)? {
            match parse_snapshot(&cached.body) {
                Ok(snapshot) if !snapshot.table.is_empty() => {
                    pricing.status = PricingStatus {
                        status: "cached".into(),
                        fetched_at: Some(cached.fetched_at),
                        snapshot_date: snapshot.snapshot_date,
                        source: cached.source_url,
                        known_models: snapshot.table.len() as i64,
                        overrides: 0,
                    };
                    pricing.fallback = Some(std::mem::replace(&mut pricing.table, snapshot.table));
                }
                _ => {}
            }
        }
        let overrides = list_price_overrides(db)?;
        pricing.status.overrides = overrides.len() as i64;
        pricing.overrides = overrides
            .into_iter()
            .map(|override_| (override_.model.trim().to_owned(), override_.rate()))
            .collect();
        Ok(pricing)
    }

    pub fn status(&self) -> &PricingStatus {
        &self.status
    }

    pub fn table(&self) -> &RateTable {
        &self.table
    }

    fn table_rate(&self, model: &str) -> Option<ModelRate> {
        self.table
            .lookup_rate(model)
            .or_else(|| self.fallback.as_ref()?.lookup_rate(model))
    }

    fn override_for(&self, model: &str) -> Option<ModelRate> {
        self.overrides.get(model.trim()).copied()
    }

    /// The rate that would price `model`: a user override first, then the
    /// table.
    pub fn lookup_rate(&self, model: &str) -> Option<ModelRate> {
        self.override_for(model).or_else(|| self.table_rate(model))
    }

    /// Prices one request. An override beats a reported cost (the user said
    /// so), a reported cost beats the table, and no rate means unpriced —
    /// never a zero dressed up as a price. Reasoning tokens are not charged:
    /// they are already inside `output_tokens`.
    pub fn price(
        &self,
        model: Option<&str>,
        tokens: &TokenUsage,
        reported_cost_microusd: Option<i64>,
    ) -> PricedUsage {
        let override_ = model.and_then(|model| self.override_for(model));
        if override_.is_none() {
            if let Some(reported) = reported_cost_microusd {
                return PricedUsage {
                    cost_microusd: Some(reported),
                    cost_source: CostSource::ProviderReported,
                };
            }
        }
        let rate = override_.or_else(|| model.and_then(|model| self.table_rate(model)));
        match rate {
            Some(rate) => PricedUsage {
                cost_microusd: Some(cost_microusd(&rate, tokens)),
                cost_source: CostSource::ModelPriced,
            },
            None => PricedUsage {
                cost_microusd: None,
                cost_source: CostSource::Unpriced,
            },
        }
    }

    /// What the cached input would have cost at the full input rate minus
    /// what it did cost. Zero when the model has no rate.
    pub fn cache_savings(&self, model: Option<&str>, tokens: &TokenUsage) -> i64 {
        let Some(rate) = model.and_then(|model| self.lookup_rate(model)) else {
            return 0;
        };
        let cache_read = tokens.cache_read_tokens.unwrap_or(0).max(0) as i128;
        let saved = cache_read * (rate.input as i128 - rate.cache_read as i128);
        round_micro(saved)
    }
}

/// Σ tokens × rate, in micro-USD, rounded half up once on the total.
pub fn cost_microusd(rate: &ModelRate, tokens: &TokenUsage) -> i64 {
    let count = |value: Option<i64>| value.unwrap_or(0).max(0) as i128;
    let total = count(tokens.uncached_input_tokens) * rate.input as i128
        + count(tokens.cache_read_tokens) * rate.cache_read as i128
        + count(tokens.cache_write_tokens) * rate.cache_write as i128
        + count(tokens.output_tokens) * rate.output as i128;
    round_micro(total)
}

fn round_micro(total: i128) -> i64 {
    let rounded = if total >= 0 {
        (total + MICRO_PER_MILLION / 2) / MICRO_PER_MILLION
    } else {
        -((-total + MICRO_PER_MILLION / 2) / MICRO_PER_MILLION)
    };
    rounded.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// USD as a float from a provider frame → micro-USD, half-up.
pub fn usd_to_microusd(value: f64) -> Option<i64> {
    value.is_finite().then(|| (value * 1_000_000.0).round() as i64)
}

// --- store ------------------------------------------------------------------

pub fn list_price_overrides(db: &Connection) -> Result<Vec<PriceOverride>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT model,input_microusd_per_mtok,output_microusd_per_mtok,cache_read_microusd_per_mtok,cache_write_microusd_per_mtok,updated_at
         FROM usage_price_overrides ORDER BY model",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PriceOverride {
            model: row.get(0)?,
            input_microusd_per_mtok: row.get(1)?,
            output_microusd_per_mtok: row.get(2)?,
            cache_read_microusd_per_mtok: row.get(3)?,
            cache_write_microusd_per_mtok: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_price_override(
    db: &Connection,
    model: &str,
    input_microusd_per_mtok: i64,
    output_microusd_per_mtok: i64,
    cache_read_microusd_per_mtok: Option<i64>,
    cache_write_microusd_per_mtok: Option<i64>,
) -> Result<PriceOverride, BridgeError> {
    let model = model.trim();
    if model.is_empty() {
        return Err(BridgeError::Invalid("a price override needs a model id".into()));
    }
    if [Some(input_microusd_per_mtok), Some(output_microusd_per_mtok), cache_read_microusd_per_mtok, cache_write_microusd_per_mtok]
        .into_iter()
        .flatten()
        .any(|rate| rate < 0)
    {
        return Err(BridgeError::Invalid("price override rates cannot be negative".into()));
    }
    let updated_at = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO usage_price_overrides(model,input_microusd_per_mtok,output_microusd_per_mtok,cache_read_microusd_per_mtok,cache_write_microusd_per_mtok,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(model) DO UPDATE SET input_microusd_per_mtok=excluded.input_microusd_per_mtok,
             output_microusd_per_mtok=excluded.output_microusd_per_mtok,
             cache_read_microusd_per_mtok=excluded.cache_read_microusd_per_mtok,
             cache_write_microusd_per_mtok=excluded.cache_write_microusd_per_mtok,
             updated_at=excluded.updated_at",
        params![
            model,
            input_microusd_per_mtok,
            output_microusd_per_mtok,
            cache_read_microusd_per_mtok,
            cache_write_microusd_per_mtok,
            updated_at
        ],
    )?;
    Ok(PriceOverride {
        model: model.into(),
        input_microusd_per_mtok,
        output_microusd_per_mtok,
        cache_read_microusd_per_mtok,
        cache_write_microusd_per_mtok,
        updated_at,
    })
}

/// Returns whether an override existed.
pub fn clear_price_override(db: &Connection, model: &str) -> Result<bool, BridgeError> {
    Ok(db.execute(
        "DELETE FROM usage_price_overrides WHERE model=?1",
        params![model.trim()],
    )? == 1)
}

#[derive(Debug, Clone)]
struct CachedRates {
    fetched_at: String,
    source_url: String,
    body: String,
}

fn rate_cache(db: &Connection) -> Result<Option<CachedRates>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT fetched_at,source_url,body FROM usage_rate_cache WHERE id=1",
            [],
            |row| {
                Ok(CachedRates {
                    fetched_at: row.get(0)?,
                    source_url: row.get(1)?,
                    body: row.get(2)?,
                })
            },
        )
        .optional()?)
}

fn store_rate_cache(db: &Connection, cached: &CachedRates) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO usage_rate_cache(id,fetched_at,source_url,body) VALUES(1,?1,?2,?3)
         ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at, source_url=excluded.source_url, body=excluded.body",
        params![cached.fetched_at, cached.source_url, cached.body],
    )?;
    Ok(())
}

/// Fetches the raw LiteLLM table. The only network call in this module, and
/// it runs only when a client asks for it.
pub fn fetch_rate_document() -> Result<Value, BridgeError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("bridge-usage-pricing")
        .build()
        .map_err(|error| BridgeError::Invalid(format!("rate fetch client: {error}")))?;
    let document: Value = client
        .get(RATE_SOURCE_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(|error| BridgeError::Invalid(format!("rate table fetch failed: {error}")))?;
    Ok(document)
}

/// Fetch, project, and cache in one step — for hosts that hold no lock.
pub fn refresh_rates(db: &Connection) -> Result<PricingStatus, BridgeError> {
    refresh_rates_from_document(db, &fetch_rate_document()?)
}

/// The refresh with the fetch factored out, so the projection and cache path
/// are testable without a network.
pub fn refresh_rates_from_document(
    db: &Connection,
    document: &Value,
) -> Result<PricingStatus, BridgeError> {
    let now = Utc::now();
    let body = project_litellm_document(document, &now.format("%Y-%m-%d").to_string());
    let snapshot = parse_snapshot(&body)?;
    if snapshot.table.is_empty() {
        return Err(BridgeError::Invalid(
            "the fetched rate table named no model Bridge can price; keeping the current table".into(),
        ));
    }
    let cached = CachedRates {
        fetched_at: now.to_rfc3339(),
        source_url: RATE_SOURCE_URL.into(),
        body,
    };
    store_rate_cache(db, &cached)?;
    let overrides = list_price_overrides(db)?.len() as i64;
    Ok(PricingStatus {
        status: "fresh".into(),
        fetched_at: Some(cached.fetched_at),
        snapshot_date: snapshot.snapshot_date,
        source: cached.source_url,
        known_models: snapshot.table.len() as i64,
        overrides,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use serde_json::json;

    fn tokens(uncached: i64, cache_read: i64, cache_write: i64, output: i64) -> TokenUsage {
        TokenUsage {
            uncached_input_tokens: Some(uncached),
            cache_read_tokens: Some(cache_read),
            cache_write_tokens: Some(cache_write),
            output_tokens: Some(output),
            ..TokenUsage::default()
        }
    }

    /// `(name, input, output, cache_read, cache_write)`.
    type Entry<'a> = (&'a str, i64, i64, Option<i64>, Option<i64>);

    fn table(entries: &[Entry<'_>]) -> RateTable {
        RateTable::from_raw(
            &entries
                .iter()
                .map(|(name, input, output, cache_read, cache_write)| {
                    (
                        (*name).to_owned(),
                        RawRate {
                            input: *input,
                            output: *output,
                            cache_read: *cache_read,
                            cache_write: *cache_write,
                        },
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn the_bundled_table_parses_and_every_entry_has_input_and_output_rates() {
        let snapshot = bundled();
        assert!(snapshot.table.len() > 100, "{} models", snapshot.table.len());
        assert!(!snapshot.snapshot_date.is_empty());
        for (name, rate) in snapshot.table.rates() {
            assert!(rate.input > 0 && rate.output > 0, "{name} has no usable rate: {rate:?}");
            assert!(rate.cache_read >= 0 && rate.cache_write >= 0, "{name}: {rate:?}");
        }
        // The ids Bridge's own catalog names are priceable.
        for model in ["claude-opus-4-6", "gpt-5", "gpt-5-codex", "o3", "gpt-4.1", "gemini/gemini-2.5-pro"] {
            assert!(snapshot.table.lookup_rate(model).is_some(), "{model} is unpriced");
        }
        assert!(BUNDLED_RATES_JSON.len() < 150 * 1024);
    }

    #[test]
    fn lookup_strips_variants_lowercases_and_aliases_unambiguous_bare_names() {
        let table = table(&[
            ("claude-opus-4-6", 5_000_000, 25_000_000, Some(500_000), Some(6_250_000)),
            ("xai/grok-4", 3_000_000, 15_000_000, None, None),
            ("dashscope/qwen3-max", 1_000, 2_000, None, None),
            ("zai/qwen3-max", 9_000, 9_000, None, None),
        ]);
        let opus = table.lookup_rate("claude-opus-4-6").unwrap();
        assert_eq!(table.lookup_rate("Claude-Opus-4-6[1m]"), Some(opus));
        assert_eq!(table.lookup_rate("anthropic/claude-opus-4-6"), Some(opus));
        // A qualified-only entry aliases its bare name when nothing conflicts…
        assert_eq!(table.lookup_rate("grok-4").unwrap().input, 3_000_000);
        // …and stays unresolved when two providers disagree.
        assert_eq!(table.lookup_rate("qwen3-max"), None);
        assert_eq!(table.lookup_rate("dashscope/qwen3-max").unwrap().input, 1_000);
        for family in ["opus", "sonnet", "haiku", "fable", "<synthetic>", "anthropic/opus", ""] {
            assert_eq!(table.lookup_rate(family), None, "{family} must never price");
        }
        // Missing cache rates fall back to the input rate.
        let grok = table.lookup_rate("xai/grok-4").unwrap();
        assert_eq!((grok.cache_read, grok.cache_write), (grok.input, grok.input));
    }

    #[test]
    fn price_follows_override_then_reported_then_table() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let pricing = Pricing::load(&db).unwrap();
        assert_eq!(pricing.status().status, "bundled");
        let usage = tokens(1_000, 0, 0, 100);

        let reported = pricing.price(Some("claude-opus-4-6"), &usage, Some(1_234));
        assert_eq!(reported, PricedUsage { cost_microusd: Some(1_234), cost_source: CostSource::ProviderReported });

        let priced = pricing.price(Some("claude-opus-4-6"), &usage, None);
        assert_eq!(priced.cost_source, CostSource::ModelPriced);
        // 1 000 × $5/M + 100 × $25/M = 5 000 + 2 500 µUSD.
        assert_eq!(priced.cost_microusd, Some(7_500));

        let unpriced = pricing.price(Some("mystery-model"), &usage, None);
        assert_eq!(unpriced, PricedUsage { cost_microusd: None, cost_source: CostSource::Unpriced });
        assert_eq!(pricing.price(None, &usage, None).cost_source, CostSource::Unpriced);

        set_price_override(&db, "claude-opus-4-6", 1_000_000, 2_000_000, None, None).unwrap();
        let pricing = Pricing::load(&db).unwrap();
        assert_eq!(pricing.status().overrides, 1);
        let overridden = pricing.price(Some("claude-opus-4-6"), &usage, Some(1_234));
        assert_eq!(overridden.cost_source, CostSource::ModelPriced);
        assert_eq!(overridden.cost_microusd, Some(1_000 + 200));
        assert!(clear_price_override(&db, "claude-opus-4-6").unwrap());
        assert!(!clear_price_override(&db, "claude-opus-4-6").unwrap());
    }

    #[test]
    fn cache_savings_are_read_tokens_times_the_discount() {
        let pricing = Pricing::bundled_only();
        let rate = pricing.lookup_rate("claude-opus-4-6").unwrap();
        let usage = tokens(0, 2_000_000, 0, 0);
        assert_eq!(
            pricing.cache_savings(Some("claude-opus-4-6"), &usage),
            2 * (rate.input - rate.cache_read)
        );
        assert_eq!(pricing.cache_savings(Some("mystery-model"), &usage), 0);
        assert_eq!(pricing.cache_savings(None, &usage), 0);
    }

    #[test]
    fn micro_usd_arithmetic_is_exact_integer_math() {
        let rate = ModelRate { input: 3_000_000, output: 0, cache_read: 0, cache_write: 0 };
        assert_eq!(cost_microusd(&rate, &tokens(1_000_000, 0, 0, 0)), 3_000_000);
        // Half-up: 1 token at $0.0000015 = 1.5 µUSD → 2.
        let tiny = ModelRate { input: 1_500_000, output: 0, cache_read: 0, cache_write: 0 };
        assert_eq!(cost_microusd(&tiny, &tokens(1, 0, 0, 0)), 2);
        assert_eq!(usd_to_microusd(0.012345), Some(12_345));
        assert_eq!(usd_to_microusd(f64::NAN), None);
    }

    #[test]
    fn a_refresh_projects_the_litellm_document_and_reports_cached_afterwards() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let document = json!({
            "claude-opus-4-6": {"litellm_provider":"anthropic","mode":"chat","input_cost_per_token":5e-6,"output_cost_per_token":25e-6,"cache_read_input_token_cost":5e-7,"cache_creation_input_token_cost":6.25e-6},
            "gpt-5": {"litellm_provider":"openai","mode":"responses","input_cost_per_token":1.25e-6,"output_cost_per_token":1e-5},
            "gpt-5-audio": {"litellm_provider":"openai","mode":"chat","input_cost_per_token":1.0,"output_cost_per_token":1.0},
            "us.anthropic.claude-opus-4-6-v1": {"litellm_provider":"bedrock","mode":"chat","input_cost_per_token":5e-6,"output_cost_per_token":25e-6},
            "half-priced": {"litellm_provider":"openai","mode":"chat","input_cost_per_token":1e-6}
        });
        let status = refresh_rates_from_document(&db, &document).unwrap();
        assert_eq!(status.status, "fresh");
        assert_eq!(status.known_models, 2);
        assert!(status.fetched_at.is_some());

        let pricing = Pricing::load(&db).unwrap();
        assert_eq!(pricing.status().status, "cached");
        assert_eq!(pricing.status().known_models, 2);
        assert_eq!(pricing.lookup_rate("claude-opus-4-6").unwrap().cache_write, 6_250_000);
        assert_eq!(pricing.lookup_rate("gpt-5").unwrap().cache_read, 1_250_000);
        assert!(pricing.lookup_rate("gpt-5-audio").is_none());
        // A model newer than the refresh still prices from the bundled table.
        assert_eq!(pricing.lookup_rate("claude-opus-5-5[1m]").unwrap().input, 4_000_000);
        let usage = tokens(1_000_000, 0, 0, 0);
        assert_eq!(pricing.price(Some("claude-opus-5-5"), &usage, None).cost_microusd, Some(4_000_000));

        let empty = refresh_rates_from_document(&db, &json!({}));
        assert!(empty.is_err(), "an empty table never replaces a working one");
        assert_eq!(Pricing::load(&db).unwrap().status().known_models, 2);
    }
}
