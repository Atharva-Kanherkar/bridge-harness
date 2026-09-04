//! The model-profiles domain: which provider, model, and effort each role runs
//! on.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::Effort;

/// The role a profile fills. Mirrors
/// `bridge_core::model_profiles::ProfilePurpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePurpose {
    StandardOrchestrator,
    PremiumOrchestrator,
    Planner,
    Implementer,
    Verifier,
    Reviewer,
    Research,
    Documentation,
    Evaluator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSelectionMode {
    TrackStandard,
    Pinned,
}

/// A profile as the operator edits it, before the server stamps a version.
/// Mirrors `bridge_core::model_profiles::ModelProfileDraft`, including its
/// refusal of unknown fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfileDraft {
    pub purpose: ProfilePurpose,
    pub provider: String,
    pub model: String,
    pub effort: Effort,
    /// Purpose to fall back to when this one cannot run.
    #[serde(default)]
    pub fallback_purpose: Option<ProfilePurpose>,
    /// Optional for compatibility with clients that predate explicit tracking.
    #[serde(default)]
    pub selection_mode: Option<ProfileSelectionMode>,
    /// Pinned profiles are exempt from learned routing.
    pub pinned: bool,
    pub learning_enabled: bool,
    #[serde(default)]
    pub budget_preference: Option<String>,
    #[serde(default)]
    pub latency_preference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveModelProfilesParams {
    /// The complete profile set; saving replaces what is stored.
    pub profiles: Vec<ModelProfileDraft>,
}

/// `models/recommended_model_profiles`' result: a bare array on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RecommendedModelProfilesResult(pub Vec<ModelProfileDraft>);

// ---------------------------------------------------------------------------
// Inline suggestions: the composer's draft-completion typeahead.
// ---------------------------------------------------------------------------

/// The composer typeahead's configuration. A small standalone blob rather than
/// a `ProfilePurpose` — that enum is closed and mirrored across three layers
/// with role semantics ("what runs the orchestrator") that a keystroke-driven
/// draft completion does not share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuggestionSettings {
    /// Off by default: nothing about the user's draft reaches a model until
    /// they opt in.
    pub enabled: bool,
    pub provider: String,
    pub model: String,
}

/// `models/get_suggestion_settings`' and `models/save_suggestion_settings`'
/// result. `configured` mirrors the Work settings pattern: `false` is a fresh
/// install reading defaults, `true` is a user who has saved this at least once
/// (including saving it switched off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionSettingsSnapshot {
    pub configured: bool,
    pub settings: SuggestionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSuggestionSettingsParams {
    pub settings: SuggestionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuggestCompletionParams {
    /// The composer's current, unsent draft text, verbatim. Sent to the model
    /// only because and while the setting above is enabled.
    pub text: String,
}

/// Why the configured suggestion model was skipped in favour of the fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionFallbackReason {
    UnknownModel,
    Unauthorized,
    RateLimited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestCompletionResult {
    /// The suggested continuation of `text`, or empty when the model had
    /// nothing to add. Never includes the draft itself.
    pub suggestion: String,
    pub used_fallback: bool,
    pub fallback_reason: Option<SuggestionFallbackReason>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    fn draft() -> ModelProfileDraft {
        ModelProfileDraft {
            purpose: ProfilePurpose::StandardOrchestrator,
            provider: "codex".into(),
            model: "gpt-5".into(),
            effort: Effort::High,
            fallback_purpose: Some(ProfilePurpose::PremiumOrchestrator),
            selection_mode: Some(ProfileSelectionMode::TrackStandard),
            pinned: false,
            learning_enabled: true,
            budget_preference: None,
            latency_preference: None,
        }
    }

    #[test]
    fn profile_drafts_round_trip_with_snake_case_purposes() {
        let save = SaveModelProfilesParams { profiles: vec![draft()] };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(wire["profiles"][0]["purpose"], json!("standard_orchestrator"));
        assert_eq!(wire["profiles"][0]["fallbackPurpose"], json!("premium_orchestrator"));
        assert_eq!(wire["profiles"][0]["effort"], json!("high"));
        assert_eq!(wire["profiles"][0]["selectionMode"], json!("track_standard"));
        assert_eq!(wire["profiles"][0]["learningEnabled"], json!(true));
        assert_eq!(round_trip(&save), save);
    }

    #[test]
    fn model_params_reject_incomplete_and_misspelled_payloads() {
        assert!(serde_json::from_value::<SaveModelProfilesParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<ModelProfileDraft>(json!({
                "purpose": "planner", "provider": "codex", "model": "gpt-5", "effort": "high",
                "pinned": false,
            }))
            .is_err(),
            "learningEnabled is required"
        );
        assert!(
            serde_json::from_value::<ModelProfileDraft>(json!({
                "purpose": "planner", "provider": "codex", "model": "gpt-5", "effort": "high",
                "pinned": false, "learning_enabled": true,
            }))
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<ModelProfileDraft>(json!({
                "purpose": "orchestrator", "provider": "codex", "model": "gpt-5",
                "effort": "high", "pinned": false, "learningEnabled": true,
            }))
            .is_err(),
            "unknown purposes must be rejected"
        );
        assert!(
            serde_json::from_value::<ModelProfileDraft>(json!({
                "purpose": "planner", "provider": "codex", "model": "gpt-5", "effort": "high",
                "pinned": false, "learningEnabled": true, "temperature": 0.7,
            }))
            .is_err(),
            "a profile draft refuses fields it does not define"
        );
    }

    fn suggestion_settings() -> SuggestionSettings {
        SuggestionSettings { enabled: true, provider: "claude".into(), model: "sonnet".into() }
    }

    #[test]
    fn suggestion_settings_round_trip_and_reject_unknown_fields() {
        let params = SaveSuggestionSettingsParams { settings: suggestion_settings() };
        let wire = serde_json::to_value(&params).unwrap();
        assert_eq!(wire["settings"]["enabled"], json!(true));
        assert_eq!(wire["settings"]["provider"], json!("claude"));
        assert_eq!(round_trip(&params), params);
        assert!(
            serde_json::from_value::<SuggestionSettings>(json!({
                "enabled": false, "provider": "claude", "model": "haiku", "extra": 1,
            }))
            .is_err(),
            "suggestion settings refuse fields they do not define"
        );
        assert!(serde_json::from_value::<SaveSuggestionSettingsParams>(json!({})).is_err());
    }

    #[test]
    fn suggest_completion_params_require_text_and_reject_unknown_fields() {
        assert!(serde_json::from_value::<SuggestCompletionParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SuggestCompletionParams>(json!({ "text": "hi", "extra": 1 })).is_err());
        let params = SuggestCompletionParams { text: "Let's ship".into() };
        assert_eq!(round_trip(&params), params);
    }

    #[test]
    fn suggest_completion_result_carries_the_fallback_reason_snake_cased() {
        let result = SuggestCompletionResult {
            suggestion: " the release".into(),
            used_fallback: true,
            fallback_reason: Some(SuggestionFallbackReason::RateLimited),
        };
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["usedFallback"], json!(true));
        assert_eq!(wire["fallbackReason"], json!("rate_limited"));
        assert_eq!(round_trip(&result), result);
    }
}
