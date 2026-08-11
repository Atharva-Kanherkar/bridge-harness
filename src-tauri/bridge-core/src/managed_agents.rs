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

use crate::managed_payload::{
    ManagedPayloadStatus, ManagedPayloadStore, PayloadRecipe, RepairReason,
};
use crate::managed_runtime::{self, HttpsArtifactFetcher, RuntimeResolution};
use crate::BridgeError;
use bridge_protocol::messages::{
    ManagedAgentBacking, ManagedAgentInspection, ManagedAgentList, ManagedAgentOperationKind,
    ManagedAgentOperationOutcome, ManagedAgentOperationResult, ManagedAgentReceiptSummary,
    ManagedAgentStatus,
};
use rusqlite::Connection;
use std::{error::Error, fmt};

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

impl Error for ManagedAgentError {
    /// Exposes the wrapped failure, so a caller walking the chain sees the real
    /// cause rather than only this layer's summary.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            _ => None,
        }
    }
}

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
            ManagedAgentError::UnknownAgent { .. } => ErrorCode::UnknownAgent,
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

fn receipt_of(
    status: &ManagedPayloadStatus,
) -> Option<&crate::managed_payload::ManagedPayloadReceipt> {
    match status {
        ManagedPayloadStatus::Installed { receipt, .. } => Some(receipt),
        ManagedPayloadStatus::NotInstalled | ManagedPayloadStatus::Repairable { .. } => None,
    }
}

/// How a payload condition maps to a repair reason, for error reporting.
fn repair_reason(status: &ManagedPayloadStatus) -> Option<RepairReason> {
    match status {
        ManagedPayloadStatus::Repairable { reason } => Some(*reason),
        ManagedPayloadStatus::NotInstalled | ManagedPayloadStatus::Installed { .. } => None,
    }
}

/// What Bridge would actually launch for this agent, and where it came from.
///
/// This is the read path the contract promises: a runtime on PATH shows as
/// `external` and is reported, not hidden, because a user needs to see the copy
/// they already have before deciding to install a managed one.
fn resolution_of(agent_id: &str, store: &ManagedPayloadStore) -> Option<RuntimeResolution> {
    managed_runtime::resolve_runtime(
        agent_id,
        None,
        store,
        &[],
        crate::binary::resolve(agent_id),
    )
    .ok()
}

fn backing_of(resolution: Option<&RuntimeResolution>) -> ManagedAgentBacking {
    match resolution {
        Some(RuntimeResolution::Managed(_)) => ManagedAgentBacking::Managed,
        Some(RuntimeResolution::Explicit(_)) => ManagedAgentBacking::Explicit,
        Some(RuntimeResolution::Bundled(_)) => ManagedAgentBacking::Bundled,
        Some(RuntimeResolution::External(_)) => ManagedAgentBacking::External,
        None => ManagedAgentBacking::None,
    }
}

/// Resolve one agent's status from the three independent facts: what Bridge owns,
/// what would launch, and where that came from.
fn status_of(agent_id: &str) -> Result<ManagedAgentStatus> {
    let label = label_for(agent_id)?;
    let store = store()?;
    let payload = store.status(agent_id).map_err(ManagedAgentError::Runtime)?;
    let resolution = resolution_of(agent_id, &store);
    let backing = backing_of(resolution.as_ref());

    // Ownership and launchability are separate questions. Bridge owns a drifted
    // payload — it is removable — but would not launch it, so `backing` describes
    // the copy that would run while `removable` describes what Bridge owns.
    let owns_payload = !matches!(payload, ManagedPayloadStatus::NotInstalled);
    let state = match (&payload, &resolution) {
        (ManagedPayloadStatus::Repairable { .. }, _) => "repairable",
        (ManagedPayloadStatus::Installed { .. }, Some(RuntimeResolution::Managed(_))) => "ready",
        (ManagedPayloadStatus::Installed { .. }, _) => "installed",
        (ManagedPayloadStatus::NotInstalled, Some(_)) => "external",
        (ManagedPayloadStatus::NotInstalled, None) => "not_installed",
    };

    Ok(ManagedAgentStatus {
        agent_id: agent_id.to_owned(),
        label: label.to_owned(),
        state: state.to_owned(),
        backing,
        removable: owns_payload,
        executable: resolution
            .as_ref()
            .map(|resolution| resolution.path().display().to_string()),
        version: receipt_of(&payload).map(|receipt| receipt.version.clone()),
        vendor_message: None,
        process_id: None,
        consecutive_failures: 0,
        last_failure: None,
    })
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
        // Reported so a user can see the copy they already have. Visible is not
        // removable: uninstall refuses this path with its own code.
        external_runtime: external_candidate(agent_id),
    })
}

/// Is a live provider process running for this agent?
///
/// Reads the tracked adapter pids the session supervisor records, and confirms
/// each is really alive by its recorded OS identity — a stale row must not make
/// an agent permanently un-removable. This is what stops a payload being deleted
/// out from under a running session.
fn live_process_for(db: &Connection, agent_id: &str) -> std::result::Result<bool, BridgeError> {
    let mut statement = db.prepare(
        "SELECT adapter_pid, adapter_process_identity FROM sessions \
         WHERE harness=?1 AND adapter_pid IS NOT NULL",
    )?;
    let rows = statement
        .query_map([agent_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().any(|(pid, identity)| {
        let pid = u32::try_from(pid).unwrap_or(0);
        match (pid, identity) {
            (0, _) => false,
            (pid, Some(identity)) => {
                crate::adapters::process_identity(pid).as_deref() == Some(identity.as_str())
            }
            (pid, None) => crate::adapters::process_identity(pid).is_some(),
        }
    }))
}

fn result_of(
    agent_id: &str,
    kind: ManagedAgentOperationKind,
    outcome: ManagedAgentOperationOutcome,
) -> Result<ManagedAgentOperationResult> {
    Ok(ManagedAgentOperationResult {
        agent_id: agent_id.to_owned(),
        kind,
        outcome,
        status: status_of(agent_id)?,
    })
}

/// Translate a payload-engine failure into the condition a caller can act on.
///
/// A blanket "not permitted" would have made `CorruptReceipt` and the 1000-range
/// I/O codes unreachable on the destructive path, which is exactly where a caller
/// most needs to know which of the three it is hitting.
fn classify(agent_id: &str, error: BridgeError, receipt: Option<RepairReason>) -> ManagedAgentError {
    let message = error.to_string();
    if let Some(reason) = receipt {
        return ManagedAgentError::CorruptReceipt {
            agent_id: agent_id.to_owned(),
            reason,
        };
    }
    if message.contains("receipt") {
        return ManagedAgentError::CorruptReceipt {
            agent_id: agent_id.to_owned(),
            reason: RepairReason::ReceiptChainMismatch,
        };
    }
    if message.contains("integrity") {
        return ManagedAgentError::IntegrityFailure {
            agent_id: agent_id.to_owned(),
            detail: message,
        };
    }
    // Anything else keeps its own code rather than being recast.
    ManagedAgentError::Runtime(error)
}

/// Fetch, verify, and install an agent's pinned payload.
///
/// Runs to completion before returning. Slow for a large closure, and honest: the
/// result says what happened rather than handing back an id for a job that does
/// not exist.
fn perform_install(agent_id: &str, repair: bool) -> Result<ManagedAgentOperationOutcome> {
    let store = store()?;
    let source = recipe_for(agent_id)?;
    let existing = store.status(agent_id).map_err(ManagedAgentError::Runtime)?;
    if !repair && matches!(existing, ManagedPayloadStatus::Installed { .. }) {
        if let Some(receipt) = receipt_of(&existing) {
            if receipt.version == pinned_version(&source) {
                return Ok(ManagedAgentOperationOutcome::AlreadyCurrent);
            }
        }
    }

    let staging = store
        .root()
        .join(".staging-fetch")
        .join(format!("{agent_id}-{}", pinned_version(&source)));
    let staged = managed_runtime::prepare(&source, &staging, &HttpsArtifactFetcher).map_err(
        |(stage, error)| match stage {
            managed_runtime::PrepareStage::Integrity => ManagedAgentError::IntegrityFailure {
                agent_id: agent_id.to_owned(),
                detail: error.to_string(),
            },
            _ => ManagedAgentError::Runtime(error),
        },
    )?;

    let recipe = PayloadRecipe {
        agent_id: agent_id.to_owned(),
        version: pinned_version(&source),
        platform: managed_runtime::npm_platform_suffix(platform_naming(agent_id))
            .ok_or_else(|| ManagedAgentError::UnsupportedPlatform {
                agent_id: agent_id.to_owned(),
            })?
            .to_owned(),
        source: source_label(&source),
        expected_sha256: staged.sha256.clone(),
        source_path: staged.source_path.clone(),
        shape: staged.shape,
        entrypoint: staged.entrypoint.clone(),
    };

    let outcome = if repair {
        store
            .repair(&recipe)
            .map(|_| ManagedAgentOperationOutcome::Repaired)
    } else {
        store
            .install(&recipe)
            .map(|_| ManagedAgentOperationOutcome::Installed)
    };
    let _ = std::fs::remove_dir_all(&staging);
    outcome.map_err(|error| classify(agent_id, error, repair_reason(&existing)))
}

fn pinned_version(source: &managed_runtime::RuntimeSource) -> String {
    match source {
        managed_runtime::RuntimeSource::NpmClosure { version, .. } => version.clone(),
        managed_runtime::RuntimeSource::ReleaseArtifact { sha256, .. } => sha256[..12].to_owned(),
    }
}

fn source_label(source: &managed_runtime::RuntimeSource) -> String {
    match source {
        managed_runtime::RuntimeSource::NpmClosure {
            package, version, ..
        } => format!("npm:{package}@{version}"),
        managed_runtime::RuntimeSource::ReleaseArtifact { url, .. } => url.clone(),
    }
}

fn platform_naming(agent_id: &str) -> managed_runtime::PlatformNaming {
    if agent_id == "opencode" {
        managed_runtime::PlatformNaming::WindowsSpelled
    } else {
        managed_runtime::PlatformNaming::NodePlatform
    }
}

/// Install an agent's managed payload from its pinned recipe.
pub fn install_managed_agent(agent_id: &str) -> Result<ManagedAgentOperationResult> {
    label_for(agent_id)?;
    let outcome = perform_install(agent_id, false)?;
    result_of(agent_id, ManagedAgentOperationKind::Install, outcome)
}

/// Repair a drifted managed payload through the engine's receipt-proven path.
pub fn repair_managed_agent(agent_id: &str) -> Result<ManagedAgentOperationResult> {
    label_for(agent_id)?;
    let outcome = perform_install(agent_id, true)?;
    result_of(agent_id, ManagedAgentOperationKind::Repair, outcome)
}

/// Remove a managed payload.
///
/// Refuses a runtime Bridge does not own, and refuses while a provider process is
/// still alive for this agent: deleting the tree under a running session is the
/// failure this whole epic exists to prevent.
pub fn uninstall_managed_agent(db: &Connection, agent_id: &str) -> Result<ManagedAgentOperationResult> {
    label_for(agent_id)?;
    let store = store()?;
    let payload = store.status(agent_id).map_err(ManagedAgentError::Runtime)?;

    if matches!(payload, ManagedPayloadStatus::NotInstalled) {
        // Nothing of Bridge's here. If a runtime is nonetheless resolvable it is
        // the user's, and saying so is more useful than "already absent".
        if let Some(candidate) = external_candidate(agent_id) {
            return Err(ManagedAgentError::ExternalNotManaged {
                agent_id: agent_id.to_owned(),
                candidate,
            });
        }
        return result_of(
            agent_id,
            ManagedAgentOperationKind::Uninstall,
            ManagedAgentOperationOutcome::AlreadyAbsent,
        );
    }

    if live_process_for(db, agent_id).map_err(ManagedAgentError::Runtime)? {
        return Err(ManagedAgentError::Busy {
            agent_id: agent_id.to_owned(),
        });
    }

    store
        .uninstall(agent_id)
        .map_err(|error| classify(agent_id, error, repair_reason(&payload)))?;
    result_of(
        agent_id,
        ManagedAgentOperationKind::Uninstall,
        ManagedAgentOperationOutcome::Removed,
    )
}

/// A user-managed runtime, if one is resolvable without a managed payload.
fn external_candidate(agent_id: &str) -> Option<String> {
    let store = managed_runtime::managed_root().map(ManagedPayloadStore::new)?;
    match resolution_of(agent_id, &store)? {
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
        // Refused on identity alone, so a bad id never reaches the filesystem —
        // and under its own code, so a client can tell it from a real failure.
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for error in [
            install_managed_agent("not-an-agent").unwrap_err(),
            repair_managed_agent("not-an-agent").unwrap_err(),
            uninstall_managed_agent(&db, "not-an-agent").unwrap_err(),
        ] {
            assert!(
                matches!(error, ManagedAgentError::UnknownAgent { .. }),
                "unexpected error: {error}"
            );
            assert_eq!(ErrorCode::from(&error), ErrorCode::UnknownAgent);
            assert_eq!(ErrorCode::from(&error).code(), 3007);
        }
    }

    #[test]
    fn a_live_provider_process_blocks_removal() {
        // The destructive path must refuse while something is still running
        // against the payload, and refuse under `AgentBusy` so a client can say
        // why rather than guessing from text.
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, harness TEXT, adapter_pid INTEGER, \
             adapter_process_identity TEXT);",
        )
        .unwrap();

        // No rows: nothing is running.
        assert!(!live_process_for(&db, "codex").unwrap());

        // This process is genuinely alive, and its recorded identity matches.
        let pid = std::process::id();
        let identity = crate::adapters::process_identity(pid);
        db.execute(
            "INSERT INTO sessions(id,harness,adapter_pid,adapter_process_identity) \
             VALUES('s1','codex',?1,?2)",
            rusqlite::params![i64::from(pid), identity],
        )
        .unwrap();
        assert!(
            live_process_for(&db, "codex").unwrap(),
            "a live tracked process must block removal"
        );
        // A different agent is unaffected.
        assert!(!live_process_for(&db, "claude").unwrap());

        // A stale row must not make an agent permanently un-removable: the
        // recorded identity no longer matches the pid.
        db.execute(
            "UPDATE sessions SET adapter_process_identity='a different process entirely'",
            [],
        )
        .unwrap();
        assert!(
            !live_process_for(&db, "codex").unwrap(),
            "a stale row must not block removal forever"
        );
    }

    #[test]
    fn store_failures_keep_their_own_condition() {
        // A blanket "not permitted" would have made these unreachable on the one
        // path where a caller most needs to tell them apart.
        let corrupt = classify(
            "codex",
            BridgeError::Invalid("managed payload uninstall refused corrupt active receipt".into()),
            None,
        );
        assert_eq!(ErrorCode::from(&corrupt), ErrorCode::CorruptReceipt);

        let drifted = classify(
            "codex",
            BridgeError::Invalid("staged integrity mismatch".into()),
            None,
        );
        assert_eq!(ErrorCode::from(&drifted), ErrorCode::IntegrityFailure);

        // Anything else keeps its own 1000-range code rather than being recast.
        let io = classify(
            "codex",
            BridgeError::Io(std::io::Error::other("disk went away")),
            None,
        );
        assert_eq!(ErrorCode::from(&io), ErrorCode::Io);
        assert!(io.source().is_some(), "the real cause stays reachable");

        // A known repair reason is reported as itself.
        let reason = classify("codex", BridgeError::Invalid("x".into()), Some(RepairReason::IntegrityDrift));
        assert!(matches!(
            reason,
            ManagedAgentError::CorruptReceipt {
                reason: RepairReason::IntegrityDrift,
                ..
            }
        ));
    }
}
