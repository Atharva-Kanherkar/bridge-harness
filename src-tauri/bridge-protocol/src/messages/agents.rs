//! The agents domain: whether an agent's *runtime* is installed at all.
//!
//! Deliberately not the `marketplace` domain, which is about plugins and
//! connectors running *inside* an agent. These answer different questions, have
//! different ownership rules, and fail in different ways — a plugin install
//! cannot leave a half-owned payload on disk, and a runtime uninstall must never
//! touch a user's own copy.
//!
//! This domain carries no installation or ownership logic of its own. It is a
//! typed surface over the payload engine, the lifecycle coordinator, and runtime
//! resolution.
//!
//! # Authentication boundary
//!
//! No method, param, or field here concerns credentials. A vendor that needs a
//! login surfaces as [`ManagedAgentStatus::vendor_message`] carrying the vendor's
//! own text, and there is nothing to submit, store, refresh, or clear.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Params for every single-agent method in the domain.
///
/// One struct per method rather than one shared struct, because the contract
/// requires a params type named after its method — a client predicts
/// `InstallManagedAgentParams` from `agents/install_managed_agent` without
/// consulting a table. They are identical by design today; declaring them
/// separately is what lets any one of them gain a field later without silently
/// widening the others.
macro_rules! agent_ref_params {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            pub struct $name {
                pub agent_id: String,
            }
        )*
    };
}

agent_ref_params![
    InspectManagedAgentParams,
    InstallManagedAgentParams,
    RepairManagedAgentParams,
    UninstallManagedAgentParams,
    StartManagedAgentParams,
    StopManagedAgentParams,
];

/// What backs the runtime Bridge would launch.
///
/// Separate from lifecycle state because the two are independent: an agent can be
/// `ready` backed by a managed payload or by the user's own copy, and only the
/// first is Bridge's to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentBacking {
    /// A Bridge-managed, receipt-bound payload.
    Managed,
    /// A user-managed runtime Bridge can see but must never remove.
    External,
    /// A path the user configured explicitly.
    Explicit,
    /// A copy shipped inside the app bundle.
    Bundled,
    /// Nothing resolvable.
    None,
}

impl ManagedAgentBacking {
    /// Is this Bridge's to uninstall?
    ///
    /// The single place the API answers that question, so a client never has to
    /// infer removability from a state string.
    pub const fn is_removable(self) -> bool {
        matches!(self, Self::Managed)
    }
}

/// A bounded, redacted record of the last failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentFailure {
    /// Already redacted and length-bounded by the lifecycle layer.
    pub context: String,
    /// Which consecutive attempt this was, 1-based.
    pub attempt: u32,
}

/// What a receipt says, reduced to what a client can display.
///
/// Deliberately omits `ownedPaths` and `entrypoint`: those authorize deletion,
/// and a UI has no use for them. Absent entirely for a runtime Bridge does not
/// own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentReceiptSummary {
    pub schema_version: u32,
    pub agent_id: String,
    pub version: String,
    pub platform: String,
    /// Where the installed bytes came from.
    pub source: String,
    pub integrity_sha256: String,
    pub installation_id: String,
    pub installed_at: String,
}

/// One integration's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentStatus {
    pub agent_id: String,
    pub label: String,
    /// The lifecycle state's stable wire string.
    pub state: String,
    pub backing: ManagedAgentBacking,
    /// True when Bridge holds a receipt for this agent's payload. Redundant with
    /// `backing` by construction and asserted so, because "can I remove this"
    /// must never be a guess a client makes from a string.
    pub removable: bool,
    /// The version of the copy that would actually launch, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// A vendor-owned message, such as a login requirement, carried verbatim.
    /// Never a Bridge failure and never credential state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_message: Option<String>,
    /// Process id when one is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    pub consecutive_failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<ManagedAgentFailure>,
}

/// Every built-in integration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentList {
    pub agents: Vec<ManagedAgentStatus>,
}

/// A managed payload and a detected external runtime, reported separately so a
/// caller can always tell which one it is looking at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentInspection {
    pub status: ManagedAgentStatus,
    /// Present only when Bridge owns a payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ManagedAgentReceiptSummary>,
    /// A user-managed runtime, if one was detected. Reported so it is visible,
    /// never so it can be removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_runtime: Option<String>,
}

/// Which lifecycle operation a progress stream belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentOperationKind {
    Install,
    Repair,
    Uninstall,
}

/// An accepted operation.
///
/// The id correlates progress notifications. It is not a completion: a client
/// that misses the terminal notification refetches authoritative state rather
/// than inferring it from progress it happened to see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentOperationStarted {
    pub operation_id: String,
    pub agent_id: String,
    pub kind: ManagedAgentOperationKind,
}

/// A stage an operation passes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentOperationStage {
    Queued,
    Fetching,
    Verifying,
    Installing,
    Stopping,
    Removing,
    Completed,
    Failed,
}

impl ManagedAgentOperationStage {
    /// Is this the last stage a client will see for the operation?
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Transient progress for one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentProgress {
    pub operation_id: String,
    pub agent_id: String,
    pub kind: ManagedAgentOperationKind,
    pub stage: ManagedAgentOperationStage,
    /// Present on a failed terminal stage. Redacted and bounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    fn status() -> ManagedAgentStatus {
        ManagedAgentStatus {
            agent_id: "codex".into(),
            label: "Codex".into(),
            state: "ready".into(),
            backing: ManagedAgentBacking::Managed,
            removable: true,
            version: Some("0.147.0".into()),
            vendor_message: None,
            process_id: None,
            consecutive_failures: 0,
            last_failure: None,
        }
    }

    #[test]
    fn agents_dtos_round_trip() {
        round_trip(&InspectManagedAgentParams {
            agent_id: "claude".into(),
        });
        round_trip(&InstallManagedAgentParams {
            agent_id: "claude".into(),
        });
        round_trip(&StopManagedAgentParams {
            agent_id: "claude".into(),
        });
        round_trip(&status());
        round_trip(&ManagedAgentList {
            agents: vec![status()],
        });
        round_trip(&ManagedAgentInspection {
            status: status(),
            receipt: Some(ManagedAgentReceiptSummary {
                schema_version: 1,
                agent_id: "codex".into(),
                version: "0.147.0".into(),
                platform: "darwin-arm64".into(),
                source: "npm:@openai/codex@0.147.0".into(),
                integrity_sha256: "a".repeat(64),
                installation_id: "b".repeat(24),
                installed_at: "2026-08-10T00:00:00+00:00".into(),
            }),
            external_runtime: Some("/opt/homebrew/bin/codex".into()),
        });
        round_trip(&ManagedAgentOperationStarted {
            operation_id: "op-1".into(),
            agent_id: "codex".into(),
            kind: ManagedAgentOperationKind::Install,
        });
        round_trip(&ManagedAgentProgress {
            operation_id: "op-1".into(),
            agent_id: "codex".into(),
            kind: ManagedAgentOperationKind::Uninstall,
            stage: ManagedAgentOperationStage::Failed,
            detail: Some("stopped responding".into()),
        });
    }

    #[test]
    fn agents_params_reject_unknown_fields() {
        // Every params type in the domain, not just one — a single lenient
        // struct would be the one a caller reaches for.
        macro_rules! assert_strict {
            ($($name:ident),*) => { $({
                assert!(serde_json::from_value::<$name>(json!({"agentId": "codex"})).is_ok());
                assert!(
                    serde_json::from_value::<$name>(json!({"agentId": "codex", "force": true}))
                        .is_err(),
                    "{} must refuse an unknown top-level field, not ignore it",
                    stringify!($name)
                );
            })* };
        }
        assert_strict!(
            InspectManagedAgentParams,
            InstallManagedAgentParams,
            RepairManagedAgentParams,
            UninstallManagedAgentParams,
            StartManagedAgentParams,
            StopManagedAgentParams
        );
    }

    #[test]
    fn only_a_managed_backing_is_removable() {
        // The API answers removability; a client must never infer it from a
        // state string, and every non-managed backing is the user's.
        assert!(ManagedAgentBacking::Managed.is_removable());
        for backing in [
            ManagedAgentBacking::External,
            ManagedAgentBacking::Explicit,
            ManagedAgentBacking::Bundled,
            ManagedAgentBacking::None,
        ] {
            assert!(
                !backing.is_removable(),
                "{backing:?} is not Bridge's to remove"
            );
        }
    }

    #[test]
    fn wire_names_are_snake_case_and_stable() {
        assert_eq!(
            serde_json::to_value(ManagedAgentBacking::External).unwrap(),
            json!("external")
        );
        assert_eq!(
            serde_json::to_value(ManagedAgentOperationKind::Uninstall).unwrap(),
            json!("uninstall")
        );
        assert_eq!(
            serde_json::to_value(ManagedAgentOperationStage::Verifying).unwrap(),
            json!("verifying")
        );
    }

    #[test]
    fn only_completed_and_failed_are_terminal_stages() {
        for stage in [
            ManagedAgentOperationStage::Completed,
            ManagedAgentOperationStage::Failed,
        ] {
            assert!(stage.is_terminal());
        }
        for stage in [
            ManagedAgentOperationStage::Queued,
            ManagedAgentOperationStage::Fetching,
            ManagedAgentOperationStage::Verifying,
            ManagedAgentOperationStage::Installing,
            ManagedAgentOperationStage::Stopping,
            ManagedAgentOperationStage::Removing,
        ] {
            assert!(!stage.is_terminal(), "{stage:?} is not terminal");
        }
    }

    #[test]
    fn no_field_in_the_domain_concerns_credentials() {
        // The whole domain's wire surface, checked in one place: installation and
        // authentication are independent, and nothing here may drift into
        // carrying a credential.
        let surface = serde_json::to_string(&ManagedAgentInspection {
            status: ManagedAgentStatus {
                vendor_message: Some("Run `codex login`.".into()),
                ..status()
            },
            receipt: None,
            external_runtime: None,
        })
        .unwrap();
        for forbidden in ["token", "apiKey", "password", "secret", "credential"] {
            assert!(
                !surface.to_lowercase().contains(&forbidden.to_lowercase()),
                "{forbidden} must not appear in the agents wire surface"
            );
        }
    }
}
