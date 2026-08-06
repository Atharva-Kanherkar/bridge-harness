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
}
