//! The automations domain: scheduled jobs each harness keeps in its own
//! native store (Claude Code's `scheduled_tasks.json`, Codex's automations
//! database), read and minimally managed through one catalog. Bridge is not
//! the scheduler — the owning app is.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whose native automation store an entry lives in. Mirrors
/// `bridge_core::automations::AutomationProvider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutomationProvider {
    Claude,
    Codex,
    OpenCode,
}

/// What to do to an automation. Mirrors
/// `bridge_core::automations::AutomationAction`. Pause/resume are only
/// honored where the native format has a paused state (Codex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutomationAction {
    Pause,
    Resume,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteAutomationActionParams {
    pub provider: AutomationProvider,
    /// The automation's id in its native store.
    pub automation_id: String,
    pub action: AutomationAction,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn automation_params_round_trip() {
        let params = ExecuteAutomationActionParams {
            provider: AutomationProvider::Codex,
            automation_id: "auto-1".into(),
            action: AutomationAction::Pause,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"provider": "codex", "automationId": "auto-1", "action": "pause"})
        );
        assert_eq!(round_trip(&params), params);
    }

    #[test]
    fn automation_params_reject_unknown_fields() {
        let error = serde_json::from_value::<ExecuteAutomationActionParams>(json!({
            "provider": "claude",
            "automationId": "task-1",
            "action": "delete",
            "extra": true,
        }))
        .unwrap_err();
        assert!(error.to_string().contains("extra"), "{error}");
    }
}
