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
    Cursor,
}

/// Native operations exposed by a provider's own automation surface. Clients
/// must use these capabilities instead of assuming providers have parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AutomationCapability {
    Create,
    Edit,
    RunNow,
    Pause,
    Resume,
    Delete,
}

/// What to do to an automation. Mirrors
/// `bridge_core::automations::AutomationAction`. Pause/resume are only
/// honored where the native format has a paused state (Codex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AutomationAction {
    Pause,
    Resume,
    RunNow,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteAutomationActionParams {
    pub provider: AutomationProvider,
    /// The automation's id in its native store.
    pub id: String,
    pub action: AutomationAction,
}

/// Create a provider-native automation when `id` is absent, or edit the
/// identified native automation when it is present. Capability checks remain
/// authoritative in bridge-core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAutomationParams {
    pub provider: AutomationProvider,
    pub id: Option<String>,
    pub prompt: String,
    pub schedule_expression: String,
    pub recurring: bool,
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
            id: "auto-1".into(),
            action: AutomationAction::Pause,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"provider": "codex", "id": "auto-1", "action": "pause"})
        );
        assert_eq!(round_trip(&params), params);
    }

    #[test]
    fn automation_params_reject_unknown_fields() {
        let error = serde_json::from_value::<ExecuteAutomationActionParams>(json!({
            "provider": "claude",
            "id": "task-1",
            "action": "delete",
            "extra": true,
        }))
        .unwrap_err();
        assert!(error.to_string().contains("extra"), "{error}");
    }

    #[test]
    fn save_automation_params_round_trip() {
        let params = SaveAutomationParams {
            provider: AutomationProvider::Claude,
            id: Some("task-1".into()),
            prompt: "Summarize CI failures".into(),
            schedule_expression: "7 9 * * 1-5".into(),
            recurring: true,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({
                "provider": "claude",
                "id": "task-1",
                "prompt": "Summarize CI failures",
                "scheduleExpression": "7 9 * * 1-5",
                "recurring": true
            })
        );
        assert_eq!(round_trip(&params), params);
    }

    #[test]
    fn capabilities_and_run_now_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_value([
                AutomationCapability::Create,
                AutomationCapability::Edit,
                AutomationCapability::RunNow,
            ])
            .unwrap(),
            json!(["create", "edit", "runNow"])
        );
        assert_eq!(serde_json::to_value(AutomationAction::RunNow).unwrap(), json!("runNow"));
        assert_eq!(serde_json::to_value(AutomationProvider::Cursor).unwrap(), json!("cursor"));
    }
}
