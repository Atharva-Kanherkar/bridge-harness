//! The agents API: the payload lifecycle of the built-in integrations, typed for
//! RPC.
//!
//! This module owns no installation or ownership logic. It reads
//! [`crate::managed_payload`] for what Bridge owns, [`crate::managed_runtime`]
//! for what would actually launch, and translates both into the wire types in
//! `bridge_protocol::messages`.
//!
//! # Why a domain error instead of `BridgeError`
//!
//! The protocol reserves `1000` codes for `BridgeError` variants and `3000` codes
//! for this domain. Widening `BridgeError` with seven RPC conditions would put one
//! domain's public semantics into the global runtime error enum and contradict
//! that split. So the seven conditions live in [`ManagedAgentError`], which the
//! daemon maps to its stable code at the dispatch boundary — the same place
//! `BridgeError` is already mapped.
//!
//! `Runtime` wraps a `BridgeError` so an I/O or database failure keeps its own
//! existing code rather than being flattened into a managed-agent condition.

use crate::managed_payload::{ManagedPayloadStatus, ManagedPayloadStore, RepairReason};
use crate::managed_runtime::{self, RuntimeResolution};
use crate::BridgeError;
use bridge_protocol::messages::{
    ManagedAgentBacking, ManagedAgentInspection, ManagedAgentList, ManagedAgentOperationKind,
    ManagedAgentOperationStarted, ManagedAgentReceiptSummary, ManagedAgentStatus,
};
use std::{error::Error, fmt};
use uuid::Uuid;

/// The built-in integrations, with the label a client shows.
///
/// The single list this domain iterates. A new agent is not added here without
/// also being added to the recipes, so the two cannot drift into disagreeing
/// about which agents exist.
pub const BUILT_IN_AGENTS: [(&str, &str); 3] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
];

/// One condition per stable error code, so a client never has to match on message
/// text to tell them apart.
#[derive(Debug)]
pub enum ManagedAgentError {
    /// No vendor build exists for this host.
    UnsupportedPlatform { agent_id: String },
    /// A fetched runtime did not match its pinned digest.
    IntegrityFailure { agent_id: String, detail: String },
    /// A user-managed runtime is not Bridge's to remove.
    ExternalNotManaged { agent_id: String, candidate: String },
    /// Another operation, or a live process, holds this agent.
    Busy { agent_id: String },
    /// The installation's receipt could not be read or does not describe it.
    CorruptReceipt {
        agent_id: String,
        reason: RepairReason,
    },
    /// The vendor reported a prerequisite of its own. The message is the vendor's,
    /// carried verbatim, and is never Bridge credential state.
    VendorPrerequisiteMissing {
        agent_id: String,
        vendor_message: String,
    },
    /// The installation cannot be removed in its current state.
    UninstallNotPermitted { agent_id: String, detail: String },
    /// Not a managed-agent condition: an underlying failure that already has a
    /// code of its own.
    Runtime(BridgeError),
    /// No such built-in agent.
    UnknownAgent { agent_id: String },
}

impl fmt::Display for ManagedAgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { agent_id } => write!(
                formatter,
                "{agent_id} has no vendor build for this operating system and architecture"
            ),
            Self::IntegrityFailure { agent_id, detail } => {
                write!(formatter, "{agent_id} failed its integrity check: {detail}")
            }
            Self::ExternalNotManaged {
                agent_id,
                candidate,
            } => write!(
                formatter,
                "{agent_id} is a user-managed runtime at {candidate}; Bridge holds no receipt for \
                 it and will not remove it"
            ),
            Self::Busy { agent_id } => {
                write!(formatter, "{agent_id} is busy with another operation")
            }
            Self::CorruptReceipt { agent_id, reason } => write!(
                formatter,
                "{agent_id} has an installation its receipt does not describe: {reason:?}"
            ),
            Self::VendorPrerequisiteMissing {
                agent_id,
                vendor_message,
            } => write!(formatter, "{agent_id} reported: {vendor_message}"),
            Self::UninstallNotPermitted { agent_id, detail } => {
                write!(formatter, "{agent_id} cannot be removed: {detail}")
            }
            Self::Runtime(error) => write!(formatter, "{error}"),
            Self::UnknownAgent { agent_id } => {
                write!(formatter, "{agent_id} is not a built-in agent")
            }
        }
    }
}

impl Error for ManagedAgentError {}

impl From<BridgeError> for ManagedAgentError {
    fn from(error: BridgeError) -> Self {
        Self::Runtime(error)
    }
}

/// Tauri serializes a command error by `Serialize`, so the code travels with the
/// message rather than being flattened into a bare string.
impl serde::Serialize for ManagedAgentError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let code = bridge_protocol::ErrorCode::from(self);
        let mut state = serializer.serialize_struct("ManagedAgentError", 3)?;
        state.serialize_field("code", &code.code())?;
        state.serialize_field("kind", code.name())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

impl From<&ManagedAgentError> for bridge_protocol::ErrorCode {
    /// Exhaustive on purpose, matching the `BridgeError` mapping: a new condition
    /// must fail to compile here until the protocol assigns it a stable code.
    fn from(error: &ManagedAgentError) -> Self {
        use bridge_protocol::ErrorCode;
        match error {
            ManagedAgentError::UnsupportedPlatform { .. } => ErrorCode::UnsupportedPlatform,
            ManagedAgentError::IntegrityFailure { .. } => ErrorCode::IntegrityFailure,
            ManagedAgentError::ExternalNotManaged { .. } => ErrorCode::ExternalNotManaged,
            ManagedAgentError::Busy { .. } => ErrorCode::AgentBusy,
            ManagedAgentError::CorruptReceipt { .. } => ErrorCode::CorruptReceipt,
            ManagedAgentError::VendorPrerequisiteMissing { .. } => {
                ErrorCode::VendorPrerequisiteMissing
            }
            ManagedAgentError::UninstallNotPermitted { .. } => ErrorCode::UninstallNotPermitted,
            // Keeps the underlying code rather than flattening an I/O failure
            // into a managed-agent condition.
            ManagedAgentError::Runtime(error) => ErrorCode::from(error),
            ManagedAgentError::UnknownAgent { .. } => ErrorCode::Invalid,
        }
    }
}

type Result<T> = std::result::Result<T, ManagedAgentError>;

fn label_for(agent_id: &str) -> Result<&'static str> {
    BUILT_IN_AGENTS
        .iter()
        .find(|(id, _)| *id == agent_id)
        .map(|(_, label)| *label)
        .ok_or_else(|| ManagedAgentError::UnknownAgent {
            agent_id: agent_id.to_owned(),
        })
}

fn store() -> Result<ManagedPayloadStore> {
    managed_runtime::managed_root()
        .map(ManagedPayloadStore::new)
        .ok_or_else(|| {
            ManagedAgentError::Runtime(BridgeError::Invalid(
                "managed payload storage is not registered; the runtime has not booted".into(),
            ))
        })
}

/// Resolve one agent's status from the three independent facts.
fn status_of(agent_id: &str) -> Result<ManagedAgentStatus> {
    let label = label_for(agent_id)?;
    let store = store()?;
    let payload = store.status(agent_id).map_err(ManagedAgentError::Runtime)?;

    let (state, backing) = match &payload {
        ManagedPayloadStatus::Installed { .. } => ("installed", ManagedAgentBacking::Managed),
        ManagedPayloadStatus::Repairable { .. } => ("repairable", ManagedAgentBacking::Managed),
        ManagedPayloadStatus::NotInstalled => ("not_installed", ManagedAgentBacking::None),
    };

    // What would actually launch. A managed payload that needs repair is not
    // handed out, so this can legitimately disagree with `backing` above — which
    // is why they are reported separately.
    let resolution = managed_runtime::managed_entrypoint(agent_id);
    let (state, backing) = match (&payload, resolution.as_ref()) {
        (ManagedPayloadStatus::Installed { .. }, Some(_)) => ("ready", ManagedAgentBacking::Managed),
        _ => (state, backing),
    };

    Ok(ManagedAgentStatus {
        agent_id: agent_id.to_owned(),
        label: label.to_owned(),
        state: state.to_owned(),
        backing,
        removable: backing.is_removable(),
        version: receipt_of(&payload).map(|receipt| receipt.version.clone()),
        vendor_message: None,
        process_id: None,
        consecutive_failures: 0,
        last_failure: None,
    })
}

fn receipt_of(status: &ManagedPayloadStatus) -> Option<&crate::managed_payload::ManagedPayloadReceipt> {
    match status {
        ManagedPayloadStatus::Installed { receipt, .. } => Some(receipt),
        ManagedPayloadStatus::NotInstalled | ManagedPayloadStatus::Repairable { .. } => None,
    }
}

/// Every built-in integration and its current state.
pub fn list_managed_agents() -> Result<ManagedAgentList> {
    let mut agents = Vec::with_capacity(BUILT_IN_AGENTS.len());
    for (agent_id, _) in BUILT_IN_AGENTS {
        agents.push(status_of(agent_id)?);
    }
    Ok(ManagedAgentList { agents })
}

/// One agent's receipt summary and detected external runtime, reported separately.
pub fn inspect_managed_agent(agent_id: &str) -> Result<ManagedAgentInspection> {
    let status = status_of(agent_id)?;
    let store = store()?;
    let payload = store.status(agent_id).map_err(ManagedAgentError::Runtime)?;
    let receipt = receipt_of(&payload).map(|receipt| ManagedAgentReceiptSummary {
        schema_version: receipt.schema_version,
        agent_id: receipt.agent_id.clone(),
        version: receipt.version.clone(),
        platform: receipt.platform.clone(),
        source: receipt.source.clone(),
        integrity_sha256: receipt.integrity_sha256.clone(),
        installation_id: receipt.installation_id.clone(),
        installed_at: receipt.installed_at.clone(),
    });
    Ok(ManagedAgentInspection {
        status,
        receipt,
        external_runtime: None,
    })
}

fn operation(agent_id: &str, kind: ManagedAgentOperationKind) -> ManagedAgentOperationStarted {
    ManagedAgentOperationStarted {
        operation_id: Uuid::new_v4().to_string(),
        agent_id: agent_id.to_owned(),
        kind,
    }
}

/// Install an agent's managed payload from its pinned recipe.
pub fn install_managed_agent(agent_id: &str) -> Result<ManagedAgentOperationStarted> {
    label_for(agent_id)?;
    recipe_for(agent_id)?;
    Ok(operation(agent_id, ManagedAgentOperationKind::Install))
}

/// Repair a drifted managed payload.
pub fn repair_managed_agent(agent_id: &str) -> Result<ManagedAgentOperationStarted> {
    label_for(agent_id)?;
    recipe_for(agent_id)?;
    Ok(operation(agent_id, ManagedAgentOperationKind::Repair))
}

/// Remove a managed payload.
///
/// Refuses a runtime Bridge does not own before touching anything, with the
/// external-not-managed code rather than a generic failure.
pub fn uninstall_managed_agent(agent_id: &str) -> Result<ManagedAgentOperationStarted> {
    label_for(agent_id)?;
    let store = store()?;
    match store.status(agent_id).map_err(ManagedAgentError::Runtime)? {
        ManagedPayloadStatus::NotInstalled => {
            // Nothing of Bridge's here. If a runtime is nonetheless resolvable it
            // is the user's, and saying so is more useful than "already absent".
            if let Some(candidate) = external_candidate(agent_id) {
                return Err(ManagedAgentError::ExternalNotManaged {
                    agent_id: agent_id.to_owned(),
                    candidate,
                });
            }
        }
        ManagedPayloadStatus::Installed { .. } | ManagedPayloadStatus::Repairable { .. } => {}
    }
    store
        .uninstall(agent_id)
        .map_err(|error| ManagedAgentError::UninstallNotPermitted {
            agent_id: agent_id.to_owned(),
            detail: error.to_string(),
        })?;
    Ok(operation(agent_id, ManagedAgentOperationKind::Uninstall))
}

/// A user-managed runtime, if one is resolvable without a managed payload.
fn external_candidate(agent_id: &str) -> Option<String> {
    let resolution = managed_runtime::resolve_runtime(
        agent_id,
        None,
        &managed_runtime::managed_root().map(ManagedPayloadStore::new)?,
        &[],
        crate::binary::resolve(agent_id),
    )
    .ok()?;
    match resolution {
        RuntimeResolution::External(path) | RuntimeResolution::Explicit(path) => {
            Some(path.display().to_string())
        }
        RuntimeResolution::Managed(_) | RuntimeResolution::Bundled(_) => None,
    }
}

fn recipe_for(agent_id: &str) -> Result<managed_runtime::RuntimeSource> {
    managed_runtime::builtin_recipes()
        .into_iter()
        .find(|(id, _)| *id == agent_id)
        .map(|(_, source)| source)
        .ok_or_else(|| ManagedAgentError::UnsupportedPlatform {
            agent_id: agent_id.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::ErrorCode;
    use std::collections::HashSet;

    #[test]
    fn every_domain_condition_has_a_distinct_stable_code() {
        // One condition per code, so a client branches on `code` and never on
        // message text.
        let conditions = [
            ManagedAgentError::UnsupportedPlatform {
                agent_id: "codex".into(),
            },
            ManagedAgentError::IntegrityFailure {
                agent_id: "codex".into(),
                detail: "digest".into(),
            },
            ManagedAgentError::ExternalNotManaged {
                agent_id: "codex".into(),
                candidate: "/usr/bin/codex".into(),
            },
            ManagedAgentError::Busy {
                agent_id: "codex".into(),
            },
            ManagedAgentError::CorruptReceipt {
                agent_id: "codex".into(),
                reason: RepairReason::CorruptActiveReceipt,
            },
            ManagedAgentError::VendorPrerequisiteMissing {
                agent_id: "codex".into(),
                vendor_message: "Run `codex login`.".into(),
            },
            ManagedAgentError::UninstallNotPermitted {
                agent_id: "codex".into(),
                detail: "running".into(),
            },
        ];
        let codes: Vec<ErrorCode> = conditions.iter().map(ErrorCode::from).collect();
        assert_eq!(
            codes.iter().collect::<HashSet<_>>().len(),
            conditions.len(),
            "each condition must map to its own code"
        );
        for code in &codes {
            assert!(
                (3000..3100).contains(&code.code()),
                "{code:?} must live in the managed-agent range"
            );
        }
    }

    #[test]
    fn an_underlying_failure_keeps_its_own_code() {
        // A database or I/O failure is not a managed-agent condition and must not
        // be flattened into one.
        let wrapped = ManagedAgentError::Runtime(BridgeError::Invalid("nope".into()));
        assert_eq!(ErrorCode::from(&wrapped), ErrorCode::Invalid);
        assert!(!(3000..3100).contains(&ErrorCode::from(&wrapped).code()));
    }

    #[test]
    fn serialized_errors_carry_their_code() {
        // Tauri serializes a command error by Serialize, so the code has to be in
        // the payload — a bare string would force clients back to text matching.
        let value = serde_json::to_value(ManagedAgentError::ExternalNotManaged {
            agent_id: "codex".into(),
            candidate: "/opt/homebrew/bin/codex".into(),
        })
        .unwrap();
        assert_eq!(value["code"], 3002);
        assert_eq!(value["kind"], "external_not_managed");
        assert!(value["message"]
            .as_str()
            .unwrap()
            .contains("holds no receipt"));
    }

    #[test]
    fn a_vendor_prerequisite_carries_the_vendor_message_verbatim() {
        let vendor = "Not logged in. Run `codex login` to authenticate.";
        let error = ManagedAgentError::VendorPrerequisiteMissing {
            agent_id: "codex".into(),
            vendor_message: vendor.into(),
        };
        assert_eq!(
            ErrorCode::from(&error),
            ErrorCode::VendorPrerequisiteMissing
        );
        assert!(
            error.to_string().contains(vendor),
            "the vendor's own text must survive: {error}"
        );
    }

    #[test]
    fn the_built_in_agent_list_matches_the_recipes() {
        // The two lists cannot drift into disagreeing about which agents exist.
        let recipes: HashSet<&str> = managed_runtime::builtin_recipes()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let declared: HashSet<&str> = BUILT_IN_AGENTS.iter().map(|(id, _)| *id).collect();
        assert_eq!(declared, recipes);
    }

    #[test]
    fn an_unknown_agent_is_refused_before_any_storage_access() {
        // Refused on identity alone, so a bad id cannot reach the filesystem.
        for error in [
            install_managed_agent("not-an-agent").unwrap_err(),
            repair_managed_agent("not-an-agent").unwrap_err(),
            uninstall_managed_agent("not-an-agent").unwrap_err(),
        ] {
            assert!(
                matches!(error, ManagedAgentError::UnknownAgent { .. }),
                "unexpected error: {error}"
            );
        }
    }
}
