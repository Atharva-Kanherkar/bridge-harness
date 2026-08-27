//! The routing domain: how much authority the learned router has over harness
//! and model selection, and how to roll a policy back.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How much authority the router has. Mirrors
/// `bridge_core::learning_router::RouterMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouterMode {
    Disabled,
    Shadow,
    Autonomous,
}

/// Per-workspace routing preferences. Mirrors
/// `bridge_core::learning_router::RouterPreferences`, including its refusal of
/// unknown fields — a misspelled preference must not silently do nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterPreferences {
    pub mode: RouterMode,
    /// Minimum expected pass rate in basis points; the server rejects values
    /// above 10000 (100%).
    #[schemars(range(min = 0, max = 10_000))]
    pub minimum_pass_bps: u16,
    #[serde(default)]
    pub pinned_harness: Option<String>,
    #[serde(default)]
    pub pinned_model: Option<String>,
    #[serde(default)]
    pub excluded_harnesses: Vec<String>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRouterPreferencesParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRouterPreferencesParams {
    pub workspace_id: String,
    pub preferences: RouterPreferences,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RollbackRoutingPolicyParams {
    pub workspace_id: String,
    /// The policy version to make active again.
    pub target_version: i64,
    /// Why the rollback happened; recorded with the policy change.
    pub explanation: String,
}

/// Whether this workspace runs a bounded model evaluation over outcomes whose
/// own result said nothing, and which judge it pins if it pins one. Mirrors
/// `bridge_core::routing_evaluation::EvaluationSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEvaluationSettings {
    pub scope_key: String,
    /// `bounded` runs the evaluator; `off` queues nothing and skips whatever is
    /// already queued.
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One evaluation run. A score is present only on a completed run: a failed or
/// skipped one carries none rather than a zero, because a zero is a judgement
/// about the delegation and neither of those reached one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEvaluationRun {
    pub run_id: String,
    pub decision_id: String,
    /// One of `queued`, `running`, `completed`, `failed`, `skipped`.
    pub status: String,
    pub harness: String,
    pub model: String,
    pub evaluator_version: String,
    /// SHA-256 of the exact bytes the verdict was reached from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_bps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_bps: Option<i64>,
    pub observed_tokens: i64,
    pub spend_microusd: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEvaluationsResult {
    pub settings: RoutingEvaluationSettings,
    pub runs: Vec<RoutingEvaluationRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRoutingEvaluationsParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetEvaluationSettingsParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateEvaluationSettingsParams {
    pub workspace_id: String,
    pub mode: String,
    /// A pinned judge is both halves or neither, and the server refuses a
    /// harness that cannot run with an empty tool scope.
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn router_preferences_round_trip_with_camel_case_wire_names() {
        let update = UpdateRouterPreferencesParams {
            workspace_id: "w-1".into(),
            preferences: RouterPreferences {
                mode: RouterMode::Autonomous,
                minimum_pass_bps: 6_500,
                pinned_harness: Some("codex".into()),
                pinned_model: None,
                excluded_harnesses: Vec::new(),
                excluded_models: vec!["haiku".into()],
            },
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(wire["workspaceId"], json!("w-1"));
        assert_eq!(wire["preferences"]["mode"], json!("autonomous"));
        assert_eq!(wire["preferences"]["minimumPassBps"], json!(6_500));
        assert_eq!(wire["preferences"]["pinnedHarness"], json!("codex"));
        assert_eq!(wire["preferences"]["excludedModels"], json!(["haiku"]));
        assert_eq!(round_trip(&update), update);

        let get = GetRouterPreferencesParams { workspace_id: "w-1".into() };
        assert_eq!(serde_json::to_value(&get).unwrap(), json!({"workspaceId": "w-1"}));
        assert_eq!(round_trip(&get), get);
    }

    #[test]
    fn optional_preferences_default_when_absent() {
        let preferences: RouterPreferences =
            serde_json::from_value(json!({"mode": "shadow", "minimumPassBps": 6_500})).unwrap();
        assert_eq!(preferences.pinned_harness, None);
        assert!(preferences.excluded_harnesses.is_empty());
    }

    #[test]
    fn rollback_params_round_trip() {
        let rollback = RollbackRoutingPolicyParams {
            workspace_id: "w".into(),
            target_version: 7,
            explanation: "canary regressed".into(),
        };
        assert_eq!(
            serde_json::to_value(&rollback).unwrap(),
            json!({"workspaceId": "w", "targetVersion": 7, "explanation": "canary regressed"})
        );
        assert_eq!(round_trip(&rollback), rollback);
    }

    #[test]
    fn evaluation_settings_round_trip_and_refuse_misspellings() {
        let update = UpdateEvaluationSettingsParams {
            workspace_id: "w".into(),
            mode: "bounded".into(),
            harness: Some("claude".into()),
            model: Some("judge".into()),
        };
        assert_eq!(
            serde_json::to_value(&update).unwrap(),
            json!({"workspaceId": "w", "mode": "bounded", "harness": "claude", "model": "judge"})
        );
        assert_eq!(round_trip(&update), update);
        assert!(serde_json::from_value::<UpdateEvaluationSettingsParams>(
            json!({"workspaceId": "w"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<UpdateEvaluationSettingsParams>(
                json!({"workspaceId": "w", "mode": "bounded", "harnesses": ["claude"]})
            )
            .is_err(),
            "a misspelled pin must be rejected, not ignored"
        );
        assert!(serde_json::from_value::<GetRoutingEvaluationsParams>(json!({})).is_err());
        assert!(serde_json::from_value::<GetEvaluationSettingsParams>(json!({})).is_err());
    }

    #[test]
    fn an_unscored_evaluation_run_keeps_its_score_off_the_wire() {
        let result = RoutingEvaluationsResult {
            settings: RoutingEvaluationSettings {
                scope_key: "workspace:w".into(),
                mode: "bounded".into(),
                harness: None,
                model: None,
            },
            runs: vec![RoutingEvaluationRun {
                run_id: "r".into(),
                decision_id: "d".into(),
                status: "failed".into(),
                harness: "claude".into(),
                model: "judge".into(),
                evaluator_version: "pinned:claude:judge".into(),
                evidence_digest: Some("abc".into()),
                score_bps: None,
                confidence_bps: None,
                observed_tokens: 900,
                spend_microusd: 1_200,
                detail: Some("The verdict is malformed".into()),
                created_at: "now".into(),
                updated_at: "now".into(),
            }],
        };
        let wire = serde_json::to_value(&result).unwrap();
        assert!(wire["runs"][0].get("scoreBps").is_none(), "a failed run is not a zero");
        assert!(wire["runs"][0].get("confidenceBps").is_none());
        assert_eq!(wire["runs"][0]["spendMicrousd"], json!(1_200));
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn routing_params_reject_incomplete_and_misspelled_payloads() {
        assert!(serde_json::from_value::<GetRouterPreferencesParams>(json!({})).is_err());
        assert!(serde_json::from_value::<UpdateRouterPreferencesParams>(
            json!({"workspaceId": "w"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<RouterPreferences>(json!({"mode": "shadow"})).is_err(),
            "minimumPassBps is required"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(
                json!({"mode": "shadow", "minimum_pass_bps": 1})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(json!({
                "mode": "shadow", "minimumPassBps": 1, "pinnedHarnesses": ["codex"],
            }))
            .is_err(),
            "a misspelled preference must be rejected, not ignored"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(
                json!({"mode": "supervised", "minimumPassBps": 1})
            )
            .is_err(),
            "unknown router modes must be rejected"
        );
        assert!(serde_json::from_value::<RollbackRoutingPolicyParams>(json!({"targetVersion": 1}))
            .is_err());
    }
}
