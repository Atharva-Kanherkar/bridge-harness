//! The skills domain: Agent Skills installed into each harness's skill root.
//! Changes are previewed first and executed against a confirmation id, so the
//! operator approves exactly what runs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whose skill root a skill is installed into. Mirrors
/// `bridge_core::skill_marketplace::SkillProvider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillProvider {
    Codex,
    Claude,
    OpenCode,
}

/// What to do to a skill. Mirrors
/// `bridge_core::skill_marketplace::SkillAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillAction {
    Install,
    Rollback,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillSuggestionsParams {
    /// Free text describing the capability the operator is looking for.
    pub query: String,
    pub provider: SkillProvider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewSkillChangeParams {
    pub skill_id: String,
    pub action: SkillAction,
    /// Every provider the change applies to.
    pub targets: Vec<SkillProvider>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteSkillChangeParams {
    /// The confirmation issued by `skills/preview_skill_change`; it expires,
    /// so an execute is always tied to a preview the operator just saw.
    pub confirmation_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn skill_params_round_trip() {
        let suggestions =
            SkillSuggestionsParams { query: "pdf".into(), provider: SkillProvider::OpenCode };
        assert_eq!(
            serde_json::to_value(&suggestions).unwrap(),
            json!({"query": "pdf", "provider": "opencode"})
        );
        assert_eq!(round_trip(&suggestions), suggestions);

        let preview = PreviewSkillChangeParams {
            skill_id: "make-pdf".into(),
            action: SkillAction::Install,
            targets: vec![SkillProvider::Codex, SkillProvider::Claude],
        };
        assert_eq!(
            serde_json::to_value(&preview).unwrap(),
            json!({"skillId": "make-pdf", "action": "install", "targets": ["codex", "claude"]})
        );
        assert_eq!(round_trip(&preview), preview);

        let execute = ExecuteSkillChangeParams { confirmation_id: "c-1".into() };
        assert_eq!(
            serde_json::to_value(&execute).unwrap(),
            json!({"confirmationId": "c-1"})
        );
        assert_eq!(round_trip(&execute), execute);
    }

    #[test]
    fn skill_params_reject_incomplete_and_unknown_payloads() {
        assert!(serde_json::from_value::<SkillSuggestionsParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SkillSuggestionsParams>(json!({"query": "pdf"})).is_err());
        assert!(
            serde_json::from_value::<SkillSuggestionsParams>(
                json!({"query": "pdf", "provider": "cursor"})
            )
            .is_err(),
            "unknown skill providers must be rejected"
        );
        assert!(serde_json::from_value::<PreviewSkillChangeParams>(
            json!({"skillId": "make-pdf", "action": "install"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<PreviewSkillChangeParams>(
                json!({"skill_id": "make-pdf", "action": "install", "targets": []})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<PreviewSkillChangeParams>(
                json!({"skillId": "make-pdf", "action": "upgrade", "targets": []})
            )
            .is_err(),
            "unknown skill actions must be rejected"
        );
        assert!(serde_json::from_value::<ExecuteSkillChangeParams>(json!({})).is_err());
    }
}
