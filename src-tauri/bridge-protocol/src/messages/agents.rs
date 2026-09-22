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
    /// True when Bridge holds a receipt for this agent's payload, so removing it
    /// is Bridge's to do. True for a payload that drifted as well: Bridge still
    /// owns those bytes and can still remove them, which is why this is not the
    /// same question as "is it healthy".
    pub removable: bool,
    /// The executable that would actually launch, when one resolves. Absent when
    /// nothing does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// The version of the copy that would actually launch, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The version Bridge currently pins for this agent, when it has a recipe
    /// for this platform. Reported next to `version` so a client can say what an
    /// update would move to instead of only that one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_version: Option<String>,
    /// True when Bridge owns this payload and the pinned recipe no longer
    /// matches what is installed.
    ///
    /// An installed payload stays launchable and receipt-valid forever, so
    /// without this a runtime pinned months ago silently keeps winning over the
    /// current pin — which is how a new provider model never appears. Only ever
    /// true for a Bridge-managed payload: a runtime the user installed is not
    /// Bridge's to version.
    pub update_available: bool,
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

/// What an operation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentOperationOutcome {
    Installed,
    Repaired,
    Removed,
    /// The payload was already installed at the pinned version.
    AlreadyCurrent,
    /// There was nothing of Bridge's to remove.
    AlreadyAbsent,
}

/// The result of a completed operation.
///
/// Deliberately not an "accepted operation" with a correlation id: these run to
/// completion before the method returns, and handing back an id that no progress
/// stream will ever reference would invite a client to wait forever. When these
/// operations move to a background job, that is a new result shape with a real
/// operation id, not a reinterpretation of this one.
///
/// Carries the post-operation status so a client does not need a second round
/// trip to learn what changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentOperationResult {
    pub agent_id: String,
    pub kind: ManagedAgentOperationKind,
    pub outcome: ManagedAgentOperationOutcome,
    pub status: ManagedAgentStatus,
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
            executable: Some("/managed/agents/codex/installations/i/payload/bin/codex".into()),
            version: Some("0.147.0".into()),
            pinned_version: Some("0.148.0".into()),
            update_available: true,
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
        round_trip(&ManagedAgentOperationResult {
            agent_id: "codex".into(),
            kind: ManagedAgentOperationKind::Install,
            outcome: ManagedAgentOperationOutcome::Installed,
            status: status(),
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
            UninstallManagedAgentParams
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
            serde_json::to_value(ManagedAgentOperationOutcome::AlreadyAbsent).unwrap(),
            json!("already_absent")
        );
    }

    #[test]
    fn an_operation_result_says_what_happened_not_that_something_started() {
        // The shape itself is the honesty check: there is no operation id to
        // correlate, because there is no stream to correlate it with.
        let value = serde_json::to_value(ManagedAgentOperationResult {
            agent_id: "codex".into(),
            kind: ManagedAgentOperationKind::Uninstall,
            outcome: ManagedAgentOperationOutcome::Removed,
            status: ManagedAgentStatus {
                backing: ManagedAgentBacking::None,
                removable: false,
                state: "not_installed".into(),
                version: None,
                executable: None,
                ..status()
            },
        })
        .unwrap();
        assert!(value.get("operationId").is_none(), "no id without a stream");
        assert_eq!(value["outcome"], json!("removed"));
        // The post-operation status travels with the result.
        assert_eq!(value["status"]["state"], json!("not_installed"));
        assert_eq!(value["status"]["removable"], json!(false));
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
