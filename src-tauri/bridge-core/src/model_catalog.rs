//! Normalized model discovery, conservative promotion, and bounded caching.
//!
//! Provider adapters only supply facts. This module owns the shared policy
//! that turns those facts into selectable models and promoted tier defaults.

use crate::{
    model::{
        CapabilityTier, ModelCatalogDiagnostics, ModelCatalogSource, ModelLifecycle, ModelOption,
    },
    BridgeError,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
};

pub const CACHE_TTL_HOURS: i64 = 6;
pub const MAX_STALE_DAYS: i64 = 7;
pub const MAX_CATALOG_ENTRIES: usize = 256;
pub const MAX_CACHE_BYTES: u64 = 512 * 1024;
const CACHE_SCHEMA_VERSION: u32 = 1;

/// The context window Bridge assumes for a model it cannot classify. This is
/// the figure the restoration projection and the context breakdown already
/// used for every model, so an unknown id changes nothing; only a recognised
/// family moves off it.
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: i64 = 128_000;

/// The incoming model's context window in tokens, by family.
///
/// Adapters do not yet report a per-model window (OpenCode's discovery sees
/// one but nothing carries it through `ModelOption`), so this is a static table
/// keyed on the model id. It exists to size what a cold start injects as
/// restoration context: too small and the new model forgets the conversation,
/// too large and the prompt compiler rejects the launch. Unknown ids and a
/// missing selection fall back to [`DEFAULT_CONTEXT_WINDOW_TOKENS`]; a bare
/// Claude alias (`opus`, `sonnet`) resolves through the harness.
pub fn context_window_tokens(harness: &str, model: Option<&str>) -> i64 {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_CONTEXT_WINDOW_TOKENS;
    };
    let lowered = model.to_ascii_lowercase();
    // OpenCode ids are `provider/model`; the family lives in the model part.
    let id = lowered.rsplit('/').next().unwrap_or(&lowered);
    if id.contains("[1m]") || id.ends_with("-1m") {
        return 1_000_000;
    }
    if id.contains("claude") {
        return 200_000;
    }
    if id.contains("gemini") || id.contains("gpt-4.1") {
        return 1_000_000;
    }
    if id.contains("gpt-4o") {
        return 128_000;
    }
    if id.contains("gpt-5") || id.contains("codex") {
        return 400_000;
    }
    if id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4") {
        return 200_000;
    }
    if id.contains("grok") {
        return 256_000;
    }
    if harness == "claude" && matches!(id, "opus" | "sonnet" | "haiku") {
        return 200_000;
    }
    DEFAULT_CONTEXT_WINDOW_TOKENS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCandidate {
    pub id: String,
    pub label: String,
    pub tier: CapabilityTier,
    pub available: bool,
    pub compatible: bool,
    pub lifecycle: ModelLifecycle,
    /// Reasoning effort levels this model accepts, carried through to the
    /// selectable `ModelOption`. Empty means the model has no effort knob.
    pub supported_effort_levels: Vec<String>,
    /// Higher values win promotion. Provider defaults should outrank ordinary
    /// discoveries; release-aware adapters may use monotonically increasing
    /// values so a newly stable model becomes Standard automatically.
    pub promotion_priority: i64,
}

impl CatalogCandidate {
    pub fn stable(
        id: impl Into<String>,
        label: impl Into<String>,
        tier: CapabilityTier,
        promotion_priority: i64,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tier,
            available: true,
            compatible: true,
            lifecycle: ModelLifecycle::Stable,
            supported_effort_levels: Vec::new(),
            promotion_priority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCatalog {
    pub models: Vec<ModelOption>,
    pub diagnostics: ModelCatalogDiagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CachedCatalog {
    schema_version: u32,
    harness_id: String,
    fetched_at: String,
    models: Vec<ModelOption>,
}

fn tier_rank(tier: CapabilityTier) -> u8 {
    match tier {
        CapabilityTier::Fast => 0,
        CapabilityTier::Standard => 1,
        CapabilityTier::Strong => 2,
    }
}

fn lifecycle_from_name(candidate: &CatalogCandidate) -> ModelLifecycle {
    let searchable = format!("{} {}", candidate.id, candidate.label).to_ascii_lowercase();
    if searchable.contains("deprecated") || searchable.contains("legacy") {
        ModelLifecycle::Deprecated
    } else if ["preview", "beta", "experimental", "nightly", "canary"]
        .iter()
        .any(|marker| searchable.contains(marker))
    {
        ModelLifecycle::Preview
    } else {
        candidate.lifecycle
    }
}

/// Normalize and promote candidates independently from their availability.
pub fn normalize(
    source: ModelCatalogSource,
    candidates: impl IntoIterator<Item = CatalogCandidate>,
) -> Vec<ModelOption> {
    let mut by_id = BTreeMap::<String, CatalogCandidate>::new();
    for mut candidate in candidates.into_iter().take(MAX_CATALOG_ENTRIES + 1) {
        candidate.id = candidate.id.trim().to_owned();
        candidate.label = candidate.label.trim().to_owned();
        if candidate.id.is_empty() || candidate.label.is_empty() {
            continue;
        }
        candidate.lifecycle = lifecycle_from_name(&candidate);
        by_id
            .entry(candidate.id.clone())
            .and_modify(|current| {
                if candidate.promotion_priority > current.promotion_priority
                    || (candidate.promotion_priority == current.promotion_priority
                        && candidate.label < current.label)
                {
                    *current = candidate.clone();
                }
            })
            .or_insert(candidate);
    }

    let mut promoted = BTreeMap::<u8, (i64, String)>::new();
    for candidate in by_id.values() {
        if candidate.available
            && candidate.compatible
            && candidate.lifecycle == ModelLifecycle::Stable
        {
            let key = tier_rank(candidate.tier);
            let contender = (candidate.promotion_priority, candidate.id.clone());
            promoted
                .entry(key)
                .and_modify(|winner| {
                    if contender.0 > winner.0 || (contender.0 == winner.0 && contender.1 < winner.1)
                    {
                        *winner = contender.clone();
                    }
                })
                .or_insert(contender);
        }
    }

    let mut models = by_id
        .into_values()
        .map(|candidate| ModelOption {
            default_for_tier: promoted
                .get(&tier_rank(candidate.tier))
                .is_some_and(|(_, id)| id == &candidate.id),
            id: candidate.id,
            label: candidate.label,
            tier: candidate.tier,
            available: candidate.available,
            compatible: candidate.compatible,
            lifecycle: candidate.lifecycle,
            source,
            supported_effort_levels: candidate.supported_effort_levels,
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        tier_rank(left.tier)
            .cmp(&tier_rank(right.tier))
            .then_with(|| {
                left.label
                    .to_ascii_lowercase()
                    .cmp(&right.label.to_ascii_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    models
}

/// Feed an already parsed provider catalog through the same normalization
/// policy. Existing provider defaults become the only entries whose lifecycle
/// can be treated as stable when the runtime supplies no lifecycle metadata.
pub fn normalize_runtime_options(models: &[ModelOption]) -> Vec<ModelOption> {
    normalize(
        ModelCatalogSource::RuntimeApi,
        models.iter().map(|model| CatalogCandidate {
            id: model.id.clone(),
            label: model.label.clone(),
            tier: model.tier,
            available: model.available,
            compatible: model.compatible,
            lifecycle: if model.default_for_tier {
                ModelLifecycle::Stable
            } else {
                model.lifecycle
            },
            supported_effort_levels: model.supported_effort_levels.clone(),
            promotion_priority: i64::from(model.default_for_tier),
        }),
    )
}

pub fn resolve(
    harness_id: &str,
    discovery: Result<Vec<CatalogCandidate>, String>,
    curated_fallback: &[CatalogCandidate],
    cache_path: Option<&Path>,
    now: DateTime<Utc>,
) -> ResolvedCatalog {
    match discovery {
        Ok(candidates) => {
            let models = normalize(ModelCatalogSource::RuntimeApi, candidates);
            if models.is_empty() {
                return failed(
                    harness_id,
                    "runtime discovery returned an empty model catalog".into(),
                    curated_fallback,
                    cache_path,
                    now,
                );
            }
            let expires_at = now + Duration::hours(CACHE_TTL_HOURS);
            let mut diagnostics = ModelCatalogDiagnostics {
                source: ModelCatalogSource::RuntimeApi,
                fetched_at: Some(now.to_rfc3339()),
                expires_at: Some(expires_at.to_rfc3339()),
                stale: false,
                last_error: None,
            };
            if let Some(path) = cache_path {
                if let Err(error) = write_cache(path, harness_id, &models, now) {
                    diagnostics.last_error = Some(error.to_string());
                }
            }
            ResolvedCatalog {
                models,
                diagnostics,
            }
        }
        Err(error) => failed(harness_id, error, curated_fallback, cache_path, now),
    }
}

fn failed(
    harness_id: &str,
    error: String,
    curated_fallback: &[CatalogCandidate],
    cache_path: Option<&Path>,
    now: DateTime<Utc>,
) -> ResolvedCatalog {
    if let Some(cached) = cache_path.and_then(|path| read_cache(path, harness_id, now).ok()) {
        let fetched_at = DateTime::parse_from_rfc3339(&cached.fetched_at)
            .expect("validated cache timestamp")
            .with_timezone(&Utc);
        let models = cached
            .models
            .into_iter()
            .map(|mut model| {
                model.source = ModelCatalogSource::LastKnownGood;
                model
            })
            .collect();
        return ResolvedCatalog {
            models,
            diagnostics: ModelCatalogDiagnostics {
                source: ModelCatalogSource::LastKnownGood,
                fetched_at: Some(fetched_at.to_rfc3339()),
                expires_at: Some((fetched_at + Duration::hours(CACHE_TTL_HOURS)).to_rfc3339()),
                stale: true,
                last_error: Some(error),
            },
        };
    }
    ResolvedCatalog {
        models: normalize(
            ModelCatalogSource::CuratedFallback,
            curated_fallback.iter().cloned(),
        ),
        diagnostics: ModelCatalogDiagnostics {
            source: ModelCatalogSource::CuratedFallback,
            fetched_at: None,
            expires_at: None,
            stale: false,
            last_error: Some(error),
        },
    }
}

fn write_cache(
    path: &Path,
    harness_id: &str,
    models: &[ModelOption],
    now: DateTime<Utc>,
) -> Result<(), BridgeError> {
    if models.len() > MAX_CATALOG_ENTRIES {
        return Err(BridgeError::Invalid(
            "model catalog exceeds entry limit".into(),
        ));
    }
    let bytes = serde_json::to_vec_pretty(&CachedCatalog {
        schema_version: CACHE_SCHEMA_VERSION,
        harness_id: harness_id.to_owned(),
        fetched_at: now.to_rfc3339(),
        models: models.to_vec(),
    })
    .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(BridgeError::Invalid(
            "model catalog cache exceeds byte limit".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| BridgeError::Invalid("model catalog cache path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| BridgeError::Io(error.error))?;
    Ok(())
}

fn read_cache(
    path: &Path,
    harness_id: &str,
    now: DateTime<Utc>,
) -> Result<CachedCatalog, BridgeError> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return Err(BridgeError::Invalid(
            "model catalog cache exceeds byte limit".into(),
        ));
    }
    let cached: CachedCatalog = serde_json::from_slice(&bytes)
        .map_err(|error| BridgeError::Invalid(format!("invalid model catalog cache: {error}")))?;
    if cached.schema_version != CACHE_SCHEMA_VERSION
        || cached.harness_id != harness_id
        || cached.models.is_empty()
        || cached.models.len() > MAX_CATALOG_ENTRIES
    {
        return Err(BridgeError::Invalid(
            "model catalog cache metadata is invalid".into(),
        ));
    }
    let fetched_at = DateTime::parse_from_rfc3339(&cached.fetched_at)
        .map_err(|_| BridgeError::Invalid("model catalog cache timestamp is invalid".into()))?
        .with_timezone(&Utc);
    if fetched_at > now || now - fetched_at > Duration::days(MAX_STALE_DAYS) {
        return Err(BridgeError::Invalid(
            "model catalog cache is outside its stale bound".into(),
        ));
    }
    Ok(cached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_tokens_resolves_known_families_and_defaults_conservatively() {
        assert_eq!(context_window_tokens("claude", Some("claude-opus-4-6")), 200_000);
        assert_eq!(context_window_tokens("claude", Some("claude-sonnet-4-5[1m]")), 1_000_000);
        assert_eq!(context_window_tokens("claude", Some("opus")), 200_000);
        assert_eq!(context_window_tokens("codex", Some("gpt-5-codex")), 400_000);
        assert_eq!(context_window_tokens("codex", Some("gpt-4o")), 128_000);
        assert_eq!(context_window_tokens("opencode", Some("google/gemini-2.5-pro")), 1_000_000);
        assert_eq!(context_window_tokens("codex", Some("o3")), 200_000);
        assert_eq!(context_window_tokens("cursor", Some("mystery-model")), DEFAULT_CONTEXT_WINDOW_TOKENS);
        assert_eq!(context_window_tokens("codex", None), DEFAULT_CONTEXT_WINDOW_TOKENS);
        assert_eq!(context_window_tokens("codex", Some("   ")), DEFAULT_CONTEXT_WINDOW_TOKENS);
    }

    fn candidate(id: &str, priority: i64) -> CatalogCandidate {
        CatalogCandidate::stable(
            id,
            id.to_ascii_uppercase(),
            CapabilityTier::Standard,
            priority,
        )
    }

    #[test]
    fn normalization_is_deterministic_and_deduplicated() {
        let left = normalize(
            ModelCatalogSource::RuntimeApi,
            [
                candidate("zeta", 1),
                candidate("alpha", 2),
                candidate("alpha", 1),
            ],
        );
        let right = normalize(
            ModelCatalogSource::RuntimeApi,
            [
                candidate("alpha", 1),
                candidate("alpha", 2),
                candidate("zeta", 1),
            ],
        );
        assert_eq!(left, right);
        assert_eq!(left.len(), 2);
        assert_eq!(
            left.iter().filter(|model| model.default_for_tier).count(),
            1
        );
    }

    #[test]
    fn promotion_requires_stable_available_compatible_models() {
        let mut preview = candidate("model-preview", 100);
        let mut deprecated = candidate("model-deprecated", 99);
        let mut inaccessible = candidate("inaccessible", 98);
        inaccessible.available = false;
        let mut incompatible = candidate("incompatible", 97);
        incompatible.compatible = false;
        preview.lifecycle = ModelLifecycle::Stable;
        deprecated.lifecycle = ModelLifecycle::Stable;
        let stable = candidate("stable", 1);
        let models = normalize(
            ModelCatalogSource::RuntimeApi,
            [preview, deprecated, inaccessible, incompatible, stable],
        );
        assert_eq!(models.len(), 5);
        assert_eq!(
            models
                .iter()
                .find(|model| model.default_for_tier)
                .map(|model| model.id.as_str()),
            Some("stable")
        );
    }

    #[test]
    fn newer_stable_discovery_can_become_the_standard() {
        let old = normalize(ModelCatalogSource::RuntimeApi, [candidate("stable-v1", 1)]);
        let refreshed = normalize(
            ModelCatalogSource::RuntimeApi,
            [candidate("stable-v1", 1), candidate("stable-v2", 2)],
        );
        assert!(old[0].default_for_tier);
        assert!(refreshed
            .iter()
            .any(|model| model.id == "stable-v2" && model.default_for_tier));
    }

    #[test]
    fn failed_discovery_uses_bounded_last_known_good_then_fallback() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("catalog.json");
        let now = Utc::now();
        let live = resolve("h", Ok(vec![candidate("live", 1)]), &[], Some(&path), now);
        assert_eq!(live.diagnostics.source, ModelCatalogSource::RuntimeApi);
        let cached = resolve(
            "h",
            Err("offline".into()),
            &[candidate("fallback", 1)],
            Some(&path),
            now + Duration::hours(1),
        );
        assert_eq!(cached.diagnostics.source, ModelCatalogSource::LastKnownGood);
        assert_eq!(cached.models[0].id, "live");
        let fallback = resolve(
            "h",
            Err("still offline".into()),
            &[candidate("fallback", 1)],
            Some(&path),
            now + Duration::days(MAX_STALE_DAYS + 1),
        );
        assert_eq!(
            fallback.diagnostics.source,
            ModelCatalogSource::CuratedFallback
        );
        assert_eq!(fallback.models[0].id, "fallback");
    }

    #[test]
    fn cache_round_trip_is_atomic_and_bounded() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("nested/catalog.json");
        let now = Utc::now();
        let resolved = resolve("h", Ok(vec![candidate("stable", 1)]), &[], Some(&path), now);
        assert!(path.is_file());
        let cached = read_cache(&path, "h", now + Duration::minutes(1)).unwrap();
        assert_eq!(cached.models, resolved.models);
        assert!(write_cache(
            &path,
            "h",
            &vec![resolved.models[0].clone(); MAX_CATALOG_ENTRIES + 1],
            now
        )
        .is_err());
        assert_eq!(read_cache(&path, "h", now).unwrap().models, resolved.models);
    }
}
