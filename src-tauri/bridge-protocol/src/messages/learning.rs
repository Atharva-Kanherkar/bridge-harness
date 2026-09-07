//! The adaptive-learning domain: scheduled learning runs, the external
//! triggers that wake them, and the approval gate before promotion.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What woke a learning run. Mirrors
/// `bridge_core::learning_job::LearningTriggerKind` — including its wire
/// spelling, so `OpenCode` is `open_code` here as it is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LearningTriggerKind {
    Manual,
    InApp,
    Codex,
    Claude,
    OpenCode,
}

/// A trigger backed by an external harness registration. Manual and in-app
/// runs never have credentials, enablement, or scheduled-task instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalLearningTriggerKind {
    Codex,
    Claude,
    OpenCode,
}

/// A trigger initiated inside Bridge. External harnesses use their registered
/// narrow commands instead of the general `run_learning` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalLearningTriggerKind {
    Manual,
    InApp,
}

/// The learning job's cadence and per-run budget. Mirrors
/// `bridge_core::learning_job::LearningSchedule`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LearningSchedule {
    pub job_id: String,
    pub enabled: bool,
    pub cadence_minutes: i64,
    pub next_run_at: Option<String>,
    pub run_budget_microusd: i64,
    pub run_budget_tokens: i64,
    /// How far a run may go on its own: `manual` recommends, `ask` waits for
    /// approval, `automatic` promotes to a guarded canary.
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetLearningStateParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunLearningParams {
    pub trigger_kind: LocalLearningTriggerKind,
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelLearningRunParams {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateLearningScheduleParams {
    pub schedule: LearningSchedule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterLearningTriggerParams {
    /// Only the external harness triggers may be registered.
    pub kind: ExternalLearningTriggerKind,
    pub registration_id: String,
    /// Reference to a stored credential; the credential itself never crosses
    /// the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetLearningTriggerInstructionsParams {
    pub kind: ExternalLearningTriggerKind,
    /// The database the external trigger should wake, quoted into the
    /// instructions verbatim.
    pub database_path: String,
    pub registration_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnableLearningTriggerParams {
    pub kind: ExternalLearningTriggerKind,
    pub registration_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveLearningRunParams {
    pub run_id: String,
}

/// `learning/get_learning_trigger_instructions`' result: the scheduled-task
/// instructions text, a bare string on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LearningTriggerInstructionsResult(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn trigger_kinds_are_snake_case_on_the_wire() {
        let run = RunLearningParams {
            trigger_kind: LocalLearningTriggerKind::InApp,
            workspace_id: "w".into(),
        };
        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            json!({"triggerKind": "in_app", "workspaceId": "w"})
        );
        assert_eq!(round_trip(&run), run);
        assert_eq!(
            serde_json::to_value(LearningTriggerKind::OpenCode).unwrap(),
            json!("open_code"),
            "the contract carries the harness's wire spelling, not its display name"
        );
        assert_eq!(
            serde_json::to_value(ExternalLearningTriggerKind::OpenCode).unwrap(),
            json!("open_code")
        );
    }

    #[test]
    fn schedules_round_trip_with_explicit_nulls() {
        let update = UpdateLearningScheduleParams {
            schedule: LearningSchedule {
                job_id: "default".into(),
                enabled: true,
                cadence_minutes: 720,
                next_run_at: None,
                run_budget_microusd: 250_000,
                run_budget_tokens: 400_000,
                mode: "ask".into(),
            },
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(
            wire["schedule"],
            json!({
                "jobId": "default",
                "enabled": true,
                "cadenceMinutes": 720,
                "nextRunAt": null,
                "runBudgetMicrousd": 250_000,
                "runBudgetTokens": 400_000,
                "mode": "ask",
            }),
            "the schedule mirrors core, which keeps an absent next run as null"
        );
        assert_eq!(round_trip(&update), update);
    }

    #[test]
    fn trigger_registration_round_trips() {
        let register = RegisterLearningTriggerParams {
            kind: ExternalLearningTriggerKind::Codex,
            registration_id: "codex-scheduled".into(),
            credential_ref: None,
            expires_at: None,
        };
        assert_eq!(
            serde_json::to_value(&register).unwrap(),
            json!({"kind": "codex", "registrationId": "codex-scheduled"}),
            "absent options stay off the wire"
        );
        assert_eq!(round_trip(&register), register);
        assert_eq!(
            serde_json::from_value::<RegisterLearningTriggerParams>(json!({
                "kind": "codex",
                "registrationId": "codex-scheduled",
                "credentialRef": null,
                "expiresAt": null,
            }))
            .unwrap(),
            register,
            "an explicit null is accepted where the field is optional"
        );

        let instructions = GetLearningTriggerInstructionsParams {
            kind: ExternalLearningTriggerKind::Claude,
            database_path: "/data/bridge.db".into(),
            registration_id: "claude-desktop".into(),
        };
        assert_eq!(
            serde_json::to_value(&instructions).unwrap(),
            json!({
                "kind": "claude",
                "databasePath": "/data/bridge.db",
                "registrationId": "claude-desktop",
            })
        );
        assert_eq!(round_trip(&instructions), instructions);

        let enable = EnableLearningTriggerParams {
            kind: ExternalLearningTriggerKind::Claude,
            registration_id: "claude-desktop".into(),
        };
        assert_eq!(round_trip(&enable), enable);
    }

    #[test]
    fn run_id_params_round_trip() {
        for wire in [
            serde_json::to_value(CancelLearningRunParams { run_id: "r-1".into() }).unwrap(),
            serde_json::to_value(ApproveLearningRunParams { run_id: "r-1".into() }).unwrap(),
        ] {
            assert_eq!(wire, json!({"runId": "r-1"}));
        }
    }

    #[test]
    fn learning_params_reject_incomplete_and_misspelled_payloads() {
        assert!(
            serde_json::from_value::<RunLearningParams>(json!({"triggerKind": "manual"})).is_err(),
            "workspaceId is required"
        );
        assert!(serde_json::from_value::<GetLearningStateParams>(json!({})).is_err());
        assert_eq!(
            serde_json::to_value(&GetLearningStateParams {
                workspace_id: "w".into()
            })
            .unwrap(),
            json!({"workspaceId": "w"})
        );
        assert!(serde_json::from_value::<RunLearningParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<RunLearningParams>(json!({"trigger_kind": "manual"})).is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<RunLearningParams>(json!({"triggerKind": "cron"})).is_err(),
            "unknown trigger kinds must be rejected"
        );
        for external in ["codex", "claude", "open_code"] {
            assert!(
                serde_json::from_value::<RunLearningParams>(json!({
                    "triggerKind": external,
                }))
                .is_err(),
                "external trigger {external} must use its registered narrow command"
            );
        }
        assert!(serde_json::from_value::<CancelLearningRunParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ApproveLearningRunParams>(json!({})).is_err());
        assert!(serde_json::from_value::<UpdateLearningScheduleParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<LearningSchedule>(json!({
                "jobId": "default", "enabled": true, "cadenceMinutes": 720,
                "runBudgetMicrousd": 1, "runBudgetTokens": 1,
            }))
            .is_err(),
            "mode is required"
        );
        assert!(serde_json::from_value::<RegisterLearningTriggerParams>(json!({"kind": "codex"}))
            .is_err());
        for internal in ["manual", "in_app"] {
            assert!(
                serde_json::from_value::<RegisterLearningTriggerParams>(json!({
                    "kind": internal,
                    "registrationId": "r",
                }))
                .is_err(),
                "internal trigger {internal} cannot be registered"
            );
            assert!(
                serde_json::from_value::<GetLearningTriggerInstructionsParams>(json!({
                    "kind": internal,
                    "databasePath": "/tmp/bridge.db",
                    "registrationId": "r",
                }))
                .is_err(),
                "internal trigger {internal} has no scheduled-task instructions"
            );
            assert!(
                serde_json::from_value::<EnableLearningTriggerParams>(json!({
                    "kind": internal,
                    "registrationId": "r",
                }))
                .is_err(),
                "internal trigger {internal} cannot be enabled"
            );
        }
        assert!(serde_json::from_value::<GetLearningTriggerInstructionsParams>(
            json!({"kind": "codex", "registrationId": "r"})
        )
        .is_err());
        assert!(serde_json::from_value::<EnableLearningTriggerParams>(json!({"kind": "codex"}))
            .is_err());
    }
}
