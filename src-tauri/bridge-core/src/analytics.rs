//! Provider-neutral, privacy-preserving facts for whole-Mac agent usage.
//!
//! This module deliberately defines no parser for a provider-owned transcript.
//! Importers added in later slices convert only exact, provider-reported numeric
//! usage into these types. Conversation bodies never enter this boundary.

use crate::BridgeError;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// How an importer can honestly describe a source's coverage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Complete,
    Partial,
    Stale,
    Unsupported,
    Unreadable,
    Empty,
}

/// Whether an importer understands a discovered history format well enough to
/// emit exact observations. Support is deliberately separate from coverage:
/// a supported source can still be partial, stale, or empty.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImporterCapability {
    Supported,
    Unsupported,
}

impl CoverageState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Stale => "stale",
            Self::Unsupported => "unsupported",
            Self::Unreadable => "unreadable",
            Self::Empty => "empty",
        }
    }
}

/// The exact interpretation used to calculate an aggregate when a provider did
/// not report its own total. `NoExactFormula` is intentional: callers must
/// leave a total unknown rather than use a tokenizer estimate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExactTotalFormula {
    ProviderReported,
    /// Codex/OpenAI input includes cached input, so cache is informational and
    /// must not be added again.
    InputIncludesCachePlusOutput,
    /// Claude's input excludes cache-read and cache-creation token counts.
    AnthropicExclusiveInputPlusCacheAndOutput,
    NoExactFormula,
}

impl Default for ExactTotalFormula {
    fn default() -> Self {
        Self::NoExactFormula
    }
}

impl ExactTotalFormula {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderReported => "provider_reported",
            Self::InputIncludesCachePlusOutput => "input_includes_cache_plus_output",
            Self::AnthropicExclusiveInputPlusCacheAndOutput => {
                "anthropic_exclusive_input_plus_cache_and_output"
            }
            Self::NoExactFormula => "no_exact_formula",
        }
    }
}

/// Mutually exclusive normalized token buckets. Optional means the provider
/// did not report that bucket; zero remains a meaningful reported value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub total_input_tokens: Option<i64>,
    pub uncached_input_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub tool_use_tokens: Option<i64>,
    pub provider_reported_total_tokens: Option<i64>,
    pub exact_total_formula: ExactTotalFormula,
}

impl TokenUsage {
    /// Returns an exact total only when it is directly reported or every input
    /// needed by the provider-specific documented formula is present.
    pub fn exact_total(&self) -> Option<i64> {
        match self.exact_total_formula {
            ExactTotalFormula::ProviderReported => self.provider_reported_total_tokens,
            ExactTotalFormula::InputIncludesCachePlusOutput => {
                checked_total([self.total_input_tokens, self.output_tokens])
            }
            ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput => checked_total([
                self.uncached_input_tokens,
                self.cache_read_tokens,
                self.cache_write_tokens,
                self.output_tokens,
            ]),
            ExactTotalFormula::NoExactFormula => None,
        }
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        for value in [
            self.total_input_tokens,
            self.uncached_input_tokens,
            self.cache_read_tokens,
            self.cache_write_tokens,
            self.output_tokens,
            self.reasoning_tokens,
            self.tool_use_tokens,
            self.provider_reported_total_tokens,
        ]
        .into_iter()
        .flatten()
        {
            if value < 0 {
                return Err(BridgeError::Invalid(
                    "agent usage token counts cannot be negative".into(),
                ));
            }
        }
        Ok(())
    }
}

fn checked_total<const N: usize>(values: [Option<i64>; N]) -> Option<i64> {
    values
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .try_fold(0_i64, |total, value| total.checked_add(value))
}

/// The only retained forward-compatible raw provider payload. It permits known
/// numeric metric names and cannot serialise arbitrary JSON or transcript text.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct NumericUsagePayload(BTreeMap<String, i64>);

impl NumericUsagePayload {
    pub fn new(values: BTreeMap<String, i64>) -> Result<Self, BridgeError> {
        let payload = Self(values);
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        for (key, value) in &self.0 {
            if !NUMERIC_USAGE_FIELDS.contains(&key.as_str()) || *value < 0 {
                return Err(BridgeError::Invalid(format!(
                    "invalid numeric agent-usage field {key:?}"
                )));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, BridgeError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| BridgeError::Invalid(error.to_string()))
    }
}

impl<'de> Deserialize<'de> for NumericUsagePayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = BTreeMap::<String, i64>::deserialize(deserializer)?;
        Self::new(values).map_err(de::Error::custom)
    }
}

const NUMERIC_USAGE_FIELDS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "cached_input_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "reasoning_tokens",
    "thoughts_token_count",
    "tool_use_tokens",
    "prompt_token_count",
    "candidates_token_count",
    "tool_use_prompt_token_count",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageObservation {
    pub source_id: String,
    pub native_record_id: String,
    pub native_session_id: Option<String>,
    pub occurred_at: String,
    pub model: Option<String>,
    pub input_semantics: String,
    pub output_semantics: String,
    pub usage: TokenUsage,
    pub numeric_usage: NumericUsagePayload,
    pub importer_version: String,
}

impl AgentUsageObservation {
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.source_id.is_empty()
            || self.native_record_id.is_empty()
            || self.occurred_at.is_empty()
            || self.importer_version.is_empty()
        {
            return Err(BridgeError::Invalid(
                "agent usage observations require stable provenance".into(),
            ));
        }
        self.usage.validate()?;
        self.numeric_usage.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAnalyticsSource {
    pub agent: String,
    pub provider: String,
    pub location: PathBuf,
    pub location_fingerprint: String,
    pub detected_version: Option<String>,
    pub capability: ImporterCapability,
    pub coverage: CoverageState,
    pub reason: Option<String>,
}

impl DiscoveredAnalyticsSource {
    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.agent.is_empty() || self.provider.is_empty() || self.location_fingerprint.is_empty()
        {
            return Err(BridgeError::Invalid(
                "analytics sources require agent, provider, and location fingerprint".into(),
            ));
        }
        if self.capability == ImporterCapability::Unsupported
            && self.coverage == CoverageState::Complete
        {
            return Err(BridgeError::Invalid(
                "an unsupported importer cannot claim complete coverage".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsScanRequest {
    pub source: DiscoveredAnalyticsSource,
    pub cursor: Option<String>,
    pub max_records: usize,
}

impl AnalyticsScanRequest {
    pub fn validate(&self) -> Result<(), BridgeError> {
        self.source.validate()?;
        if !(1..=10_000).contains(&self.max_records) {
            return Err(BridgeError::Invalid(
                "analytics scans must request between 1 and 10000 records".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsScanResult {
    pub observations: Vec<AgentUsageObservation>,
    pub attributions: Vec<AnalyticsAttribution>,
    pub next_cursor: Option<String>,
    pub records_imported: usize,
    pub records_skipped: usize,
    pub coverage: CoverageState,
    pub warning: Option<String>,
}

impl AnalyticsScanResult {
    pub fn validate_for(&self, request: &AnalyticsScanRequest) -> Result<(), BridgeError> {
        request.validate()?;
        if self.observations.len() > request.max_records {
            return Err(BridgeError::Invalid(
                "analytics importers must return bounded scan batches".into(),
            ));
        }
        if request.source.capability == ImporterCapability::Unsupported
            && (!self.observations.is_empty() || self.coverage == CoverageState::Complete)
        {
            return Err(BridgeError::Invalid(
                "unsupported analytics importers cannot emit observations or complete coverage"
                    .into(),
            ));
        }
        for observation in &self.observations {
            observation.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsAttribution {
    pub native_session_id: Option<String>,
    pub native_record_id: Option<String>,
    pub kind: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSourceProbe {
    pub detected_version: Option<String>,
    pub format_version: Option<String>,
    pub capability: ImporterCapability,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsScanProgress {
    pub records_read: usize,
    pub records_imported: usize,
    pub records_skipped: usize,
}

/// A host-owned control hook keeps parser implementations streaming and lets a
/// long first import stop without publishing a partial source as complete.
pub trait AnalyticsScanControl: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn report_progress(&self, progress: AnalyticsScanProgress);
}

/// Provider-owned history importers implement this contract. Discovery and
/// parsing stay in Rust; callers stream source records and persist only the
/// validated metadata above.
pub trait AnalyticsImporter: Send + Sync {
    fn importer_id(&self) -> &'static str;
    fn discover(&self) -> Result<Vec<DiscoveredAnalyticsSource>, BridgeError>;
    fn probe(
        &self,
        source: &DiscoveredAnalyticsSource,
    ) -> Result<AnalyticsSourceProbe, BridgeError>;
    fn scan(
        &self,
        request: AnalyticsScanRequest,
        control: &dyn AnalyticsScanControl,
    ) -> Result<AnalyticsScanResult, BridgeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_input_includes_cache_exactly_once() {
        let usage = TokenUsage {
            total_input_tokens: Some(1_000),
            cache_read_tokens: Some(900),
            output_tokens: Some(250),
            exact_total_formula: ExactTotalFormula::InputIncludesCachePlusOutput,
            ..Default::default()
        };
        assert_eq!(usage.exact_total(), Some(1_250));
    }

    #[test]
    fn anthropic_exclusive_input_adds_each_cache_bucket_once() {
        let usage = TokenUsage {
            uncached_input_tokens: Some(100),
            cache_read_tokens: Some(200),
            cache_write_tokens: Some(300),
            output_tokens: Some(400),
            exact_total_formula: ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput,
            ..Default::default()
        };
        assert_eq!(usage.exact_total(), Some(1_000));
    }

    #[test]
    fn unknown_or_incomplete_totals_are_not_estimated() {
        let unknown = TokenUsage {
            total_input_tokens: Some(100),
            output_tokens: Some(50),
            exact_total_formula: ExactTotalFormula::NoExactFormula,
            ..Default::default()
        };
        assert_eq!(unknown.exact_total(), None);

        let incomplete = TokenUsage {
            uncached_input_tokens: Some(100),
            output_tokens: Some(50),
            exact_total_formula: ExactTotalFormula::AnthropicExclusiveInputPlusCacheAndOutput,
            ..Default::default()
        };
        assert_eq!(incomplete.exact_total(), None);
    }

    #[test]
    fn numeric_payload_rejects_transcript_fields() {
        let error = NumericUsagePayload::new(BTreeMap::from([("prompt".into(), 1)])).unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid numeric agent-usage field"));
    }

    #[test]
    fn discovery_cannot_claim_complete_before_a_scan() {
        let source = DiscoveredAnalyticsSource {
            agent: "codex".into(),
            provider: "openai".into(),
            location: PathBuf::from("/private/usage.jsonl"),
            location_fingerprint: "sha256:source".into(),
            detected_version: None,
            capability: ImporterCapability::Unsupported,
            coverage: CoverageState::Complete,
            reason: None,
        };
        assert!(source.validate().is_err());
    }

    #[test]
    fn unsupported_sources_cannot_emit_exact_observations() {
        let request = AnalyticsScanRequest {
            source: DiscoveredAnalyticsSource {
                agent: "unknown-agent".into(),
                provider: "unknown-provider".into(),
                location: PathBuf::from("/private/history"),
                location_fingerprint: "sha256:source".into(),
                detected_version: None,
                capability: ImporterCapability::Unsupported,
                coverage: CoverageState::Unsupported,
                reason: Some("format is not supported".into()),
            },
            cursor: None,
            max_records: 10,
        };
        let result = AnalyticsScanResult {
            observations: vec![AgentUsageObservation {
                source_id: "source".into(),
                native_record_id: "record".into(),
                native_session_id: None,
                occurred_at: "2026-08-30T00:00:00Z".into(),
                model: None,
                input_semantics: "inclusive".into(),
                output_semantics: "delta".into(),
                usage: TokenUsage::default(),
                numeric_usage: NumericUsagePayload::default(),
                importer_version: "test".into(),
            }],
            attributions: vec![],
            next_cursor: None,
            records_imported: 0,
            records_skipped: 0,
            coverage: CoverageState::Unsupported,
            warning: Some("format is not supported".into()),
        };
        assert!(result.validate_for(&request).is_err());
    }

    #[test]
    fn numeric_payload_cannot_be_deserialized_with_transcript_content() {
        let error =
            serde_json::from_str::<NumericUsagePayload>(r#"{"response":"secret"}"#).unwrap_err();
        assert!(error.to_string().contains("invalid type"));
    }
}
