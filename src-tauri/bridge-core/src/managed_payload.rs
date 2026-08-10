//! Receipt-owned payload storage for Bridge-managed agent runtimes.
//!
//! This module deliberately has no network, RPC, UI, or vendor-authentication
//! concerns. Callers provide an already resolved artifact plus its expected
//! integrity. Bridge stages and owns only what it can prove through receipts.
//!
//! # Integrity digest
//!
//! A payload's digest describes the *logical tree* that lands in
//! `<installation>/payload`, so the same artifact digests identically on every
//! host. Entries are sorted by their canonical relative path and folded in as
//! `dir\0<path>\0` or `file\0<path>\0<len><bytes>`. Three properties matter:
//!
//!   * Paths are joined with `/` and must be valid UTF-8 — never the platform
//!     separator, and never a lossy conversion that could collide.
//!   * Directories are hashed too, so an added or removed empty directory is
//!     drift rather than an invisible change.
//!   * File bytes stream through the hasher, so memory stays flat regardless of
//!     payload size. A real runtime tree is hundreds of megabytes.
//!
//! Permission bits are deliberately *not* hashed: they do not survive every
//! transport a recipe may arrive over, and hashing them would make a digest
//! platform-specific. The one bit that matters — is the entrypoint
//! executable — is enforced on install and checked by [`ManagedPayloadStore::status`]
//! instead.
//!
//! # Serialization
//!
//! Lifecycle operations serialize per managed root and agent through an
//! in-process mutex. That is sufficient *because* [`crate::ownership`] already
//! guarantees a single owner process per data directory; callers that place a
//! managed root outside a leased data directory do not get cross-process
//! exclusion. Promotion still tolerates a lost race: a rename onto an existing
//! installation falls back to verifying and adopting it.
//!
//! Symlink rejection here is check-then-use rather than capability-based, and
//! `sync_directory` is a no-op off Unix, so the durability half of atomic
//! promotion is weaker on Windows. Both are recorded rather than hidden;
//! migrating this module onto `cap-std` (already used by
//! [`crate::workspace_files`]) is deliberately left out of this slice.

use crate::BridgeError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use uuid::Uuid;

pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

/// Staging subtree under the managed root, one directory per agent.
const STAGING_DIR_NAME: &str = ".staging";
/// Read granularity while streaming payload bytes through the hasher.
const HASH_CHUNK_BYTES: usize = 128 * 1024;
/// Ceiling on a receipt read. Receipts are small, fixed-shape JSON documents;
/// bounding the read keeps a corrupt or hostile file from being loaded whole.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadShape {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadRecipe {
    pub agent_id: String,
    pub version: String,
    pub platform: String,
    pub source: String,
    pub source_path: PathBuf,
    pub shape: PayloadShape,
    pub expected_sha256: String,
    pub entrypoint: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPayloadReceipt {
    pub schema_version: u32,
    pub agent_id: String,
    pub version: String,
    pub platform: String,
    pub source: String,
    pub integrity_sha256: String,
    pub installation_id: String,
    pub owned_paths: Vec<PathBuf>,
    pub entrypoint: PathBuf,
    pub payload_shape: PayloadShape,
    pub payload_entrypoint: PathBuf,
    pub installed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed(ManagedPayloadReceipt),
    AlreadyInstalled(ManagedPayloadReceipt),
    Recovered(ManagedPayloadReceipt),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepairReason {
    CorruptActiveReceipt,
    ActiveReceiptMismatch,
    MissingInstallation,
    CorruptEmbeddedReceipt,
    ReceiptChainMismatch,
    MissingPayload,
    MissingEntrypoint,
    EntrypointNotExecutable,
    IntegrityDrift,
    UnsafeManagedPath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedPayloadStatus {
    NotInstalled,
    Installed {
        receipt: ManagedPayloadReceipt,
        entrypoint: PathBuf,
    },
    Repairable {
        reason: RepairReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepairOutcome {
    AlreadyHealthy(ManagedPayloadReceipt),
    Repaired(ManagedPayloadReceipt),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UninstallOutcome {
    Uninstalled,
    AlreadyAbsent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalRuntimeInspection {
    pub candidate: PathBuf,
    pub available: bool,
}

impl InstallOutcome {
    pub fn receipt(&self) -> &ManagedPayloadReceipt {
        match self {
            Self::Installed(receipt)
            | Self::AlreadyInstalled(receipt)
            | Self::Recovered(receipt) => receipt,
        }
    }
}

impl ManagedPayloadReceipt {
    /// The single installation directory this receipt authorizes, relative to
    /// the managed root.
    ///
    /// `owned_paths` is a list for forward compatibility but is required to
    /// hold exactly one entry (see [`validate_receipt_ownership`]). Reading it
    /// through this accessor keeps a hand-written or truncated receipt from
    /// panicking a caller that forgot to validate first.
    pub fn owned_root(&self) -> Result<&PathBuf, RepairReason> {
        match self.owned_paths.as_slice() {
            [owned] => Ok(owned),
            _ => Err(RepairReason::ActiveReceiptMismatch),
        }
    }
}

#[derive(Debug)]
pub struct ManagedPayloadStore {
    root: PathBuf,
    #[cfg(test)]
    fail_after_promotion: std::sync::atomic::AtomicBool,
}

struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

static AGENT_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

impl ManagedPayloadStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            #[cfg(test)]
            fail_after_promotion: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn install(&self, recipe: &PayloadRecipe) -> Result<InstallOutcome, BridgeError> {
        let recipe = recipe.validate()?;
        let lock = agent_lock(&self.root, &recipe.agent_id);
        let _guard = lock_agent(&lock);
        self.install_locked(&recipe)
    }

    pub fn status(&self, agent_id: &str) -> Result<ManagedPayloadStatus, BridgeError> {
        validate_component("agent id", agent_id)?;
        let lock = agent_lock(&self.root, agent_id);
        let _guard = lock_agent(&lock);
        ensure_root_is_not_symlink(&self.root)?;
        self.status_locked(agent_id)
    }

    pub fn repair(&self, recipe: &PayloadRecipe) -> Result<RepairOutcome, BridgeError> {
        let recipe = recipe.validate()?;
        let lock = agent_lock(&self.root, &recipe.agent_id);
        let _guard = lock_agent(&lock);

        if let ManagedPayloadStatus::Installed { receipt, .. } =
            self.status_locked(&recipe.agent_id)?
        {
            if receipt_matches_recipe(&receipt, &fresh_receipt(&recipe)) {
                return Ok(RepairOutcome::AlreadyHealthy(receipt));
            }
            return Err(BridgeError::Invalid(
                "managed payload active installation does not match the requested recipe".into(),
            ));
        }

        let agent_root = self.root.join("agents").join(&recipe.agent_id);
        let active_path = agent_root.join("active.json");
        let installation_root = agent_root
            .join("installations")
            .join(&recipe.installation_id);
        let expected = fresh_receipt(&recipe);
        let active_proof = read_receipt_if_valid(&active_path)
            .is_some_and(|receipt| receipt_matches_recipe(&receipt, &expected));
        let embedded_proof = read_receipt_if_valid(&installation_root.join("receipt.json"))
            .is_some_and(|receipt| receipt_matches_recipe(&receipt, &expected));
        if !active_proof && !embedded_proof {
            return Err(BridgeError::Invalid(
                "managed payload repair refused because no valid receipt proves ownership".into(),
            ));
        }
        verify_recipe_source_integrity(&recipe)?;
        ensure_safe_managed_descendant(
            &self.root,
            &PathBuf::from("agents")
                .join(&recipe.agent_id)
                .join("installations")
                .join(&recipe.installation_id),
        )?;

        if installation_root.is_dir() && embedded_proof {
            if let Ok(receipt) =
                verify_existing_installation(&self.root, &recipe, &installation_root)
            {
                write_json_atomic(&active_path, &receipt)?;
                return Ok(RepairOutcome::Repaired(receipt));
            }
        }
        // Replacing the deterministic path means deleting whatever sits there.
        // A receipt proves Bridge *created* that path, but a corrupt embedded
        // receipt means the directory itself is no longer self-describing — so
        // require it to still look like a Bridge installation before removing
        // it. Anything carrying foreign content fails closed rather than
        // taking user files down with the repair.
        if path_exists_no_follow(&installation_root)? {
            ensure_directory_is_bridge_shaped(&installation_root)?;
        }
        remove_path_if_present(&installation_root, true)?;
        remove_path_if_present(&active_path, false)?;
        let installed = self.install_locked(&recipe)?.receipt().clone();
        Ok(RepairOutcome::Repaired(installed))
    }

    pub fn uninstall(&self, agent_id: &str) -> Result<UninstallOutcome, BridgeError> {
        validate_component("agent id", agent_id)?;
        let lock = agent_lock(&self.root, agent_id);
        let _guard = lock_agent(&lock);
        ensure_root_is_not_symlink(&self.root)?;

        let active_relative = PathBuf::from("agents").join(agent_id).join("active.json");
        ensure_safe_managed_descendant(&self.root, &active_relative)?;
        let active_path = self.root.join(&active_relative);
        let receipt = match read_receipt_strict(&active_path) {
            Ok(receipt) => receipt,
            Err(BridgeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(UninstallOutcome::AlreadyAbsent);
            }
            Err(error) => {
                return Err(BridgeError::Invalid(format!(
                    "managed payload uninstall refused corrupt active receipt: {error}"
                )));
            }
        };
        validate_receipt_ownership(&receipt, agent_id).map_err(|reason| {
            BridgeError::Invalid(format!(
                "managed payload uninstall refused unsafe receipt: {reason:?}"
            ))
        })?;
        let owned_relative = receipt.owned_root().map_err(|reason| {
            BridgeError::Invalid(format!(
                "managed payload uninstall refused unsafe receipt: {reason:?}"
            ))
        })?;
        ensure_safe_managed_descendant(&self.root, owned_relative)?;
        let installation_root = self.root.join(owned_relative);

        match fs::symlink_metadata(&installation_root) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(BridgeError::Invalid(
                        "managed payload uninstall refused a non-directory installation".into(),
                    ));
                }
                let embedded = read_receipt_strict(&installation_root.join("receipt.json"))
                    .map_err(|error| {
                        BridgeError::Invalid(format!(
                            "managed payload uninstall refused invalid embedded receipt: {error}"
                        ))
                    })?;
                if embedded != receipt {
                    return Err(BridgeError::Invalid(
                        "managed payload uninstall refused a mismatched receipt chain".into(),
                    ));
                }
                fs::remove_dir_all(&installation_root)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(BridgeError::Io(error)),
        }
        // Superseded versions of this agent are Bridge-owned too, and nothing
        // else can ever reach them once the active receipt is gone. Reclaim
        // every installation whose own receipt proves Bridge installed it for
        // this agent; unreceipted directories are left strictly alone.
        prune_owned_installations(&self.root, agent_id, None)?;
        remove_path_if_present(&active_path, false)?;
        Ok(UninstallOutcome::Uninstalled)
    }

    fn status_locked(&self, agent_id: &str) -> Result<ManagedPayloadStatus, BridgeError> {
        let active_relative = PathBuf::from("agents").join(agent_id).join("active.json");
        if ensure_safe_managed_descendant(&self.root, &active_relative).is_err() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        let active_path = self.root.join(&active_relative);
        let receipt = match read_receipt_strict(&active_path) {
            Ok(receipt) => receipt,
            Err(BridgeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManagedPayloadStatus::NotInstalled);
            }
            Err(_) => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::CorruptActiveReceipt,
                });
            }
        };
        if let Err(reason) = validate_receipt_ownership(&receipt, agent_id) {
            return Ok(ManagedPayloadStatus::Repairable { reason });
        }
        let owned_relative = match receipt.owned_root() {
            Ok(owned) => owned,
            Err(reason) => return Ok(ManagedPayloadStatus::Repairable { reason }),
        };
        if ensure_safe_managed_descendant(&self.root, owned_relative).is_err() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        let installation_root = self.root.join(owned_relative);
        let metadata = match fs::symlink_metadata(&installation_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::MissingInstallation,
                });
            }
            Err(error) => return Err(BridgeError::Io(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        let embedded = match read_receipt_strict(&installation_root.join("receipt.json")) {
            Ok(receipt) => receipt,
            Err(_) => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::CorruptEmbeddedReceipt,
                });
            }
        };
        if embedded != receipt {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::ReceiptChainMismatch,
            });
        }
        let payload = installation_root.join("payload");
        let payload_metadata = match fs::symlink_metadata(&payload) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::MissingPayload,
                });
            }
            Err(error) => return Err(BridgeError::Io(error)),
        };
        if payload_metadata.file_type().is_symlink() || !payload_metadata.is_dir() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        // The leaf checks above only cover their own final component, so walk
        // the entrypoint's own ancestry too: an intermediate directory swapped
        // for a symlink would otherwise be resolved silently.
        if ensure_safe_managed_descendant(&self.root, &receipt.entrypoint).is_err() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        let entrypoint = self.root.join(&receipt.entrypoint);
        let entrypoint_metadata = match fs::symlink_metadata(&entrypoint) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::MissingEntrypoint,
                });
            }
            Err(error) => return Err(BridgeError::Io(error)),
        };
        if entrypoint_metadata.file_type().is_symlink() || !entrypoint_metadata.is_file() {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::UnsafeManagedPath,
            });
        }
        // Mode is outside the digest, so a stripped executable bit is drift the
        // hash cannot see. Reporting `Installed` for a payload that can only
        // fail with EACCES at spawn time would make the status meaningless.
        if !path_is_executable(&entrypoint) {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::EntrypointNotExecutable,
            });
        }
        let digest = match payload_tree_digest(&payload) {
            Ok(digest) => digest,
            Err(_) => {
                return Ok(ManagedPayloadStatus::Repairable {
                    reason: RepairReason::IntegrityDrift,
                });
            }
        };
        if digest != receipt.integrity_sha256 {
            return Ok(ManagedPayloadStatus::Repairable {
                reason: RepairReason::IntegrityDrift,
            });
        }
        Ok(ManagedPayloadStatus::Installed {
            receipt,
            entrypoint,
        })
    }

    fn install_locked(&self, recipe: &ValidatedRecipe) -> Result<InstallOutcome, BridgeError> {
        ensure_root_is_not_symlink(&self.root)?;

        let agent_root = self.root.join("agents").join(&recipe.agent_id);
        let installations_root = agent_root.join("installations");
        let installation_root = installations_root.join(&recipe.installation_id);
        let active_path = agent_root.join("active.json");
        let installations_relative = PathBuf::from("agents")
            .join(&recipe.agent_id)
            .join("installations");
        ensure_safe_managed_descendant(&self.root, &installations_relative)?;
        ensure_safe_managed_descendant(
            &self.root,
            &PathBuf::from("agents")
                .join(&recipe.agent_id)
                .join("active.json"),
        )?;

        if path_exists_no_follow(&installation_root)? {
            let receipt = verify_existing_installation(&self.root, recipe, &installation_root)?;
            let was_active =
                read_receipt_if_valid(&active_path).is_some_and(|active| active == receipt);
            write_json_atomic(&active_path, &receipt)?;
            return Ok(if was_active {
                InstallOutcome::AlreadyInstalled(receipt)
            } else {
                InstallOutcome::Recovered(receipt)
            });
        }

        // Staging is per-agent so that reaping what a killed process left
        // behind cannot touch another agent's in-flight staging directory. The
        // agent lock already excludes a second staging attempt for *this*
        // agent, and agent ids cannot contain `/`, so this subtree is ours.
        let staging_relative = PathBuf::from(STAGING_DIR_NAME).join(&recipe.agent_id);
        ensure_safe_managed_descendant(&self.root, &staging_relative)?;
        let staging_root = self.root.join(&staging_relative);
        reap_staging_root(&staging_root)?;
        fs::create_dir_all(&staging_root)?;
        let staging_path = staging_root.join(Uuid::new_v4().simple().to_string());
        fs::create_dir(&staging_path)?;
        let mut staging = StagingGuard {
            path: staging_path.clone(),
            armed: true,
        };
        let staged_payload = staging_path.join("payload");
        // The source digest is deliberately *not* computed here: verifying the
        // representation actually stored is what matters, and hashing the source
        // as well would double every install's read and hash cost for no added
        // guarantee.
        copy_payload(recipe, &staged_payload)?;
        sync_tree_files(&staged_payload)?;
        let staged_entrypoint = staged_payload.join(&recipe.entrypoint);
        if !staged_entrypoint.is_file() {
            return Err(BridgeError::Invalid(format!(
                "managed payload entrypoint is not a file: {}",
                recipe.entrypoint.display()
            )));
        }
        ensure_executable(&staged_entrypoint)?;
        let staged_digest = payload_tree_digest(&staged_payload)?;
        if staged_digest != recipe.expected_sha256 {
            return Err(BridgeError::Invalid(format!(
                "managed payload staged integrity mismatch: expected {}, got {staged_digest}",
                recipe.expected_sha256
            )));
        }

        let receipt = fresh_receipt(recipe);
        write_json_atomic(&staging_path.join("receipt.json"), &receipt)?;
        sync_directory(&staging_path)?;
        // Created only once the staged tree has proven itself, so a rejected
        // payload leaves no trace of the agent under the managed root.
        fs::create_dir_all(&installations_root)?;
        match fs::rename(&staging_path, &installation_root) {
            Ok(()) => {
                staging.armed = false;
                sync_directory(&installations_root)?;
            }
            Err(_error) if path_exists_no_follow(&installation_root)? => {
                let existing =
                    verify_existing_installation(&self.root, recipe, &installation_root)?;
                write_json_atomic(&active_path, &existing)?;
                return Ok(InstallOutcome::Recovered(existing));
            }
            Err(error) => return Err(BridgeError::Io(error)),
        }

        #[cfg(test)]
        if self
            .fail_after_promotion
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(BridgeError::Invalid(
                "injected failure after managed payload promotion".into(),
            ));
        }

        write_json_atomic(&active_path, &receipt)?;
        // An upgrade supersedes whatever version was active. Nothing can reach
        // the old installation once this receipt is published, so reclaim it
        // now rather than leaving hundreds of megabytes of unreferenced runtime
        // on disk for every version the user ever installed.
        prune_owned_installations(&self.root, &recipe.agent_id, Some(&recipe.installation_id))?;
        Ok(InstallOutcome::Installed(receipt))
    }

    #[cfg(test)]
    fn inject_failure_after_promotion(&self) {
        self.fail_after_promotion
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn verify_recipe_source_integrity(recipe: &ValidatedRecipe) -> Result<(), BridgeError> {
    let actual_source_digest =
        source_digest(&recipe.source_path, recipe.shape, &recipe.entrypoint)?;
    if actual_source_digest != recipe.expected_sha256 {
        return Err(BridgeError::Invalid(format!(
            "managed payload source integrity mismatch: expected {}, got {actual_source_digest}",
            recipe.expected_sha256
        )));
    }
    Ok(())
}

/// Serialization lock for one agent under one managed root.
///
/// `Path` hashes by component, so equivalent spellings of the same root share a
/// lock. Poisoning is deliberately tolerated in both this registry and the
/// per-agent lock below: the mutex guards no data, so a panic under it leaves
/// nothing inconsistent, and mapping `PoisonError` to a hard error would brick
/// every later install, repair, and uninstall for the rest of the process.
fn agent_lock(root: &Path, agent_id: &str) -> Arc<Mutex<()>> {
    let key = root.join("agents").join(agent_id);
    let mut locks = AGENT_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn lock_agent(lock: &Arc<Mutex<()>>) -> std::sync::MutexGuard<'_, ()> {
    lock.lock().unwrap_or_else(|error| error.into_inner())
}

/// Remove every installation of `agent_id` whose own embedded receipt proves
/// Bridge installed it, except `keep`.
///
/// Ownership is re-proven per directory from its own receipt, so a directory
/// Bridge did not write — a user's own folder, a hand-made sibling, an external
/// runtime someone dropped in — is never a candidate. A single unreadable entry
/// is skipped rather than failing the caller: reclaiming disk must never be the
/// reason an install or uninstall reports failure.
fn prune_owned_installations(
    root: &Path,
    agent_id: &str,
    keep: Option<&str>,
) -> Result<(), BridgeError> {
    let installations_relative = PathBuf::from("agents").join(agent_id).join("installations");
    if ensure_safe_managed_descendant(root, &installations_relative).is_err() {
        return Ok(());
    }
    let installations_root = root.join(&installations_relative);
    let children = match fs::read_dir(&installations_root) {
        Ok(children) => children,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(BridgeError::Io(error)),
    };
    for child in children {
        let Ok(child) = child else { continue };
        let path = child.path();
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if keep == Some(name.as_str()) || validate_component("installation id", &name).is_err() {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(receipt) = read_receipt_strict(&path.join("receipt.json")) else {
            continue;
        };
        // The receipt has to describe *this* directory for this agent, or it is
        // not proof of anything.
        if validate_receipt_ownership(&receipt, agent_id).is_err()
            || receipt.installation_id != name
        {
            continue;
        }
        let _ = fs::remove_dir_all(&path);
    }
    Ok(())
}

/// Discard staging directories a previous process left behind.
///
/// Called under the agent lock, immediately before staging, so nothing live for
/// this agent can be in here. Without this, every crash between `create_dir`
/// and promotion leaks a partial payload copy that nothing ever reclaims.
fn reap_staging_root(staging_root: &Path) -> Result<(), BridgeError> {
    let children = match fs::read_dir(staging_root) {
        Ok(children) => children,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(BridgeError::Io(error)),
    };
    for child in children {
        let Ok(child) = child else { continue };
        let path = child.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let _ = if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
    }
    Ok(())
}

/// Refuse to treat a directory as a replaceable Bridge installation unless it
/// still has the shape Bridge writes: a `payload` directory, and nothing at the
/// top level except that and `receipt.json`.
///
/// This is the weaker sibling of a valid embedded receipt, used only on the
/// repair path where the receipt itself is what went bad. It keeps a corrupt
/// receipt recoverable without letting repair delete foreign content that
/// happens to occupy the deterministic path.
fn ensure_directory_is_bridge_shaped(path: &Path) -> Result<(), BridgeError> {
    let refuse = || {
        BridgeError::Invalid(format!(
            "managed payload repair refused to replace {} because it does not have the shape of a Bridge installation",
            path.display()
        ))
    };
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(refuse());
    }
    let mut has_payload = false;
    for child in fs::read_dir(path)? {
        let child = child?;
        match child.file_name().to_str() {
            Some("payload") => {
                let payload = fs::symlink_metadata(child.path())?;
                if payload.file_type().is_symlink() || !payload.is_dir() {
                    return Err(refuse());
                }
                has_payload = true;
            }
            Some("receipt.json") => {}
            _ => return Err(refuse()),
        }
    }
    if !has_payload {
        return Err(refuse());
    }
    Ok(())
}

fn ensure_root_is_not_symlink(root: &Path) -> Result<(), BridgeError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(BridgeError::Invalid(
            "managed payload root cannot be a symlink".into(),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(BridgeError::Invalid(
            "managed payload root must be a directory".into(),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(BridgeError::Io(error)),
    }
}

fn ensure_safe_managed_descendant(root: &Path, relative: &Path) -> Result<(), BridgeError> {
    validate_relative_path("managed descendant", relative)?;
    ensure_root_is_not_symlink(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(BridgeError::Invalid(
                "managed payload descendant is not a normal relative path".into(),
            ));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(BridgeError::Invalid(format!(
                    "managed payload path contains a symlink: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(BridgeError::Io(error)),
        }
    }
    Ok(())
}

fn validate_receipt_ownership(
    receipt: &ManagedPayloadReceipt,
    requested_agent: &str,
) -> Result<(), RepairReason> {
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION
        || receipt.agent_id != requested_agent
        || validate_component("receipt agent id", &receipt.agent_id).is_err()
        || validate_component("receipt version", &receipt.version).is_err()
        || validate_component("receipt platform", &receipt.platform).is_err()
        || receipt.source.trim().is_empty()
        || normalize_sha256(&receipt.integrity_sha256).is_err()
        || validate_relative_path("receipt payload entrypoint", &receipt.payload_entrypoint)
            .is_err()
    {
        return Err(RepairReason::ActiveReceiptMismatch);
    }
    let expected_installation_id = installation_id(
        &receipt.agent_id,
        &receipt.version,
        &receipt.platform,
        &receipt.integrity_sha256,
    );
    let expected_owned = PathBuf::from("agents")
        .join(&receipt.agent_id)
        .join("installations")
        .join(&expected_installation_id);
    let expected_entrypoint = expected_owned
        .join("payload")
        .join(&receipt.payload_entrypoint);
    if receipt.installation_id != expected_installation_id
        || receipt.owned_paths != [expected_owned]
        || receipt.entrypoint != expected_entrypoint
        || validate_relative_path("receipt entrypoint", &receipt.entrypoint).is_err()
    {
        return Err(RepairReason::ActiveReceiptMismatch);
    }
    Ok(())
}

/// Digest of an installed `payload` directory.
///
/// Always the full tree, for both payload shapes. Hashing only the entrypoint
/// for file-shaped payloads left everything else under `payload/` outside
/// integrity, so a planted sibling — a dylib next to a binary, say — was
/// invisible to drift detection. The tree walk is digest-compatible with the
/// file-shaped source digest, which synthesizes the same entry set.
fn payload_tree_digest(payload_root: &Path) -> Result<String, BridgeError> {
    validate_source_root(payload_root, PayloadShape::Directory)?;
    let mut entries = Vec::new();
    collect_tree_entries(payload_root, payload_root, &mut entries)?;
    hash_entries(entries)
}

fn remove_path_if_present(path: &Path, directory: bool) -> Result<(), BridgeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(BridgeError::Invalid(format!(
            "managed payload refused to remove symlink: {}",
            path.display()
        ))),
        Ok(metadata) if directory && metadata.is_dir() => {
            fs::remove_dir_all(path)?;
            Ok(())
        }
        Ok(metadata) if !directory && metadata.is_file() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(BridgeError::Invalid(format!(
            "managed payload refused to remove unexpected path type: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(BridgeError::Io(error)),
    }
}

pub fn inspect_external_runtime(candidate: impl Into<PathBuf>) -> ExternalRuntimeInspection {
    let candidate = candidate.into();
    let available = external_candidate_is_executable(&candidate);
    ExternalRuntimeInspection {
        candidate,
        available,
    }
}

#[cfg(unix)]
fn external_candidate_is_executable(candidate: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(candidate)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn external_candidate_is_executable(candidate: &Path) -> bool {
    candidate.is_file()
}

fn read_receipt_if_valid(path: &Path) -> Option<ManagedPayloadReceipt> {
    read_receipt_strict(path).ok()
}

fn read_receipt_strict(path: &Path) -> Result<ManagedPayloadReceipt, BridgeError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BridgeError::Invalid(format!(
            "managed payload receipt is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_RECEIPT_BYTES {
        return Err(BridgeError::Invalid(format!(
            "managed payload receipt is implausibly large ({} bytes): {}",
            metadata.len(),
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_RECEIPT_BYTES)
        .read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        BridgeError::Invalid(format!("managed payload receipt is corrupt: {error}"))
    })
}

fn path_exists_no_follow(path: &Path) -> Result<bool, BridgeError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(BridgeError::Io(error)),
    }
}

fn verify_existing_installation(
    root: &Path,
    recipe: &ValidatedRecipe,
    installation_root: &Path,
) -> Result<ManagedPayloadReceipt, BridgeError> {
    let metadata = fs::symlink_metadata(installation_root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BridgeError::Invalid(
            "managed payload installation target is not an owned directory".into(),
        ));
    }
    let receipt_path = installation_root.join("receipt.json");
    let receipt = read_receipt_strict(&receipt_path)?;
    let expected = fresh_receipt(recipe);
    if !receipt_matches_recipe(&receipt, &expected) {
        return Err(BridgeError::Invalid(
            "managed payload receipt does not prove ownership of the requested installation".into(),
        ));
    }
    let payload = installation_root.join("payload");
    let digest = payload_tree_digest(&payload)?;
    if digest != recipe.expected_sha256 {
        return Err(BridgeError::Invalid(
            "managed payload installation exists but its integrity has drifted".into(),
        ));
    }
    ensure_safe_managed_descendant(root, &receipt.entrypoint)?;
    let entrypoint = root.join(&receipt.entrypoint);
    if !entrypoint.is_file() {
        return Err(BridgeError::Invalid(
            "managed payload installation exists but its entrypoint is missing".into(),
        ));
    }
    if !path_is_executable(&entrypoint) {
        return Err(BridgeError::Invalid(
            "managed payload installation exists but its entrypoint is not executable".into(),
        ));
    }
    Ok(receipt)
}

/// Does this receipt describe the installation the recipe asks for?
///
/// `source` is excluded on purpose. It records where the bytes came from, not
/// which bytes they are — `integrity_sha256` already pins that, and
/// `installation_id` is derived from agent, version, platform, and integrity
/// without it. Comparing it here meant that relabelling provenance for a
/// byte-identical artifact (a mirror becoming a vendor CDN, say) mapped to the
/// same installation directory and then failed its own ownership check: install
/// and repair both errored while status still reported `Installed`, and only a
/// full uninstall could clear it. The receipt keeps the provenance of the bytes
/// as first installed, which is the accurate record.
fn receipt_matches_recipe(
    receipt: &ManagedPayloadReceipt,
    expected: &ManagedPayloadReceipt,
) -> bool {
    receipt.schema_version == RECEIPT_SCHEMA_VERSION
        && receipt.agent_id == expected.agent_id
        && receipt.version == expected.version
        && receipt.platform == expected.platform
        && receipt.integrity_sha256 == expected.integrity_sha256
        && receipt.installation_id == expected.installation_id
        && receipt.owned_paths == expected.owned_paths
        && receipt.entrypoint == expected.entrypoint
        && receipt.payload_shape == expected.payload_shape
        && receipt.payload_entrypoint == expected.payload_entrypoint
}

/// Is `path` executable by somebody?
///
/// Mirrors [`external_candidate_is_executable`] so a Bridge-managed entrypoint
/// is held to the same standard as an external runtime. Off Unix there is no
/// permission bit to read, so a regular file is as much as can be asserted.
fn path_is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Make a staged entrypoint executable.
///
/// A recipe's source may arrive from a transport that drops permission bits, and
/// mode is outside the integrity digest by design, so install asserts the one
/// bit that decides whether the installation can run at all.
fn ensure_executable(path: &Path) -> Result<(), BridgeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        let mode = permissions.mode();
        if mode & 0o111 != 0o111 {
            permissions.set_mode(mode | 0o111);
            fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedRecipe {
    agent_id: String,
    version: String,
    platform: String,
    source: String,
    source_path: PathBuf,
    shape: PayloadShape,
    expected_sha256: String,
    entrypoint: PathBuf,
    installation_id: String,
}

impl PayloadRecipe {
    fn validate(&self) -> Result<ValidatedRecipe, BridgeError> {
        for (label, value) in [
            ("agent id", self.agent_id.as_str()),
            ("version", self.version.as_str()),
            ("platform", self.platform.as_str()),
        ] {
            validate_component(label, value)?;
        }
        if self.source.trim().is_empty() || self.source.len() > 512 {
            return Err(BridgeError::Invalid(
                "managed payload source must be present and bounded".into(),
            ));
        }
        validate_relative_path("entrypoint", &self.entrypoint)?;
        let expected_sha256 = normalize_sha256(&self.expected_sha256)?;
        validate_source_root(&self.source_path, self.shape)?;
        let installation_id = installation_id(
            &self.agent_id,
            &self.version,
            &self.platform,
            &expected_sha256,
        );
        Ok(ValidatedRecipe {
            agent_id: self.agent_id.clone(),
            version: self.version.clone(),
            platform: self.platform.clone(),
            source: self.source.clone(),
            source_path: self.source_path.clone(),
            shape: self.shape,
            expected_sha256,
            entrypoint: self.entrypoint.clone(),
            installation_id,
        })
    }
}

/// Digest of a payload source, as it will exist under `payload/` once installed.
///
/// For a file-shaped source that means the entry set of a tree containing the
/// single file at `entrypoint`, including the directories `entrypoint` implies —
/// so `source_digest` of the source and [`payload_tree_digest`] of the
/// installation agree by construction.
pub fn source_digest(
    source: &Path,
    shape: PayloadShape,
    entrypoint: &Path,
) -> Result<String, BridgeError> {
    validate_relative_path("entrypoint", entrypoint)?;
    validate_source_root(source, shape)?;
    let entries = match shape {
        PayloadShape::File => {
            let length = fs::symlink_metadata(source)?.len();
            file_shape_entries(source, entrypoint, length)?
        }
        PayloadShape::Directory => {
            let mut entries = Vec::new();
            collect_tree_entries(source, source, &mut entries)?;
            entries
        }
    };
    hash_entries(entries)
}

fn validate_component(label: &str, value: &str) -> Result<(), BridgeError> {
    let safe = !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !safe {
        return Err(BridgeError::Invalid(format!(
            "managed payload {label} is not a safe path component"
        )));
    }
    Ok(())
}

fn validate_relative_path(label: &str, path: &Path) -> Result<(), BridgeError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(BridgeError::Invalid(format!(
            "managed payload {label} must be a non-empty relative path"
        )));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BridgeError::Invalid(format!(
            "managed payload {label} cannot traverse or contain path prefixes"
        )));
    }
    Ok(())
}

fn normalize_sha256(value: &str) -> Result<String, BridgeError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BridgeError::Invalid(
            "managed payload integrity must be a 64-character SHA-256 digest".into(),
        ));
    }
    Ok(normalized)
}

fn validate_source_root(source: &Path, shape: PayloadShape) -> Result<(), BridgeError> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        BridgeError::Invalid(format!(
            "managed payload source {} cannot be inspected: {error}",
            source.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(BridgeError::Invalid(
            "managed payload sources cannot be symlinks".into(),
        ));
    }
    let expected_type = match shape {
        PayloadShape::File => metadata.is_file(),
        PayloadShape::Directory => metadata.is_dir(),
    };
    if !expected_type {
        return Err(BridgeError::Invalid(
            "managed payload source does not match its declared shape".into(),
        ));
    }
    Ok(())
}

/// One entry of a payload tree: a directory, or a file to stream from `source`.
struct PayloadEntry {
    /// Canonical `/`-joined relative path.
    path: String,
    /// `None` for a directory.
    source: Option<(PathBuf, u64)>,
}

/// Render a relative path as the digest's canonical form.
///
/// Rejects non-UTF-8 rather than folding through `to_string_lossy`, where two
/// distinct names both become `U+FFFD` and collide, and joins with `/` so a
/// tree digests the same on Windows as it does on Unix.
fn canonical_relative(label: &str, relative: &Path) -> Result<String, BridgeError> {
    validate_relative_path(label, relative)?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(BridgeError::Invalid(format!(
                "managed payload {label} is not a normal relative path"
            )));
        };
        let part = part.to_str().ok_or_else(|| {
            BridgeError::Invalid(format!(
                "managed payload {label} must be valid UTF-8: {}",
                relative.display()
            ))
        })?;
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn collect_tree_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<PayloadEntry>,
) -> Result<(), BridgeError> {
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(BridgeError::Invalid(format!(
                "managed payload source contains a symlink: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            BridgeError::Invalid("managed payload source escaped its root".into())
        })?;
        let canonical = canonical_relative("artifact path", relative)?;
        if metadata.is_dir() {
            entries.push(PayloadEntry {
                path: canonical,
                source: None,
            });
            collect_tree_entries(root, &path, entries)?;
        } else if metadata.is_file() {
            entries.push(PayloadEntry {
                path: canonical,
                source: Some((path, metadata.len())),
            });
        } else {
            return Err(BridgeError::Invalid(format!(
                "managed payload source contains an unsupported filesystem entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// The entry set a single-file payload will have once staged under `payload/`.
fn file_shape_entries(
    source: &Path,
    entrypoint: &Path,
    length: u64,
) -> Result<Vec<PayloadEntry>, BridgeError> {
    let canonical = canonical_relative("entrypoint", entrypoint)?;
    let mut parts = canonical.split('/').collect::<Vec<_>>();
    let file = parts.pop().unwrap_or_default().to_owned();
    let mut entries = Vec::new();
    let mut ancestor = String::new();
    for part in parts {
        if !ancestor.is_empty() {
            ancestor.push('/');
        }
        ancestor.push_str(part);
        entries.push(PayloadEntry {
            path: ancestor.clone(),
            source: None,
        });
    }
    let path = if ancestor.is_empty() {
        file
    } else {
        format!("{ancestor}/{file}")
    };
    entries.push(PayloadEntry {
        path,
        source: Some((source.to_path_buf(), length)),
    });
    Ok(entries)
}

fn hash_entries(mut entries: Vec<PayloadEntry>) -> Result<String, BridgeError> {
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let mut digest = Sha256::new();
    for entry in &entries {
        match &entry.source {
            None => {
                digest.update(b"dir\0");
                digest.update(entry.path.as_bytes());
                digest.update(b"\0");
            }
            Some((path, length)) => {
                digest.update(b"file\0");
                digest.update(entry.path.as_bytes());
                digest.update(b"\0");
                digest.update(length.to_le_bytes());
                let streamed = stream_file_into(path, &mut digest)?;
                // The length is committed to before the bytes, so a file that
                // changes size underneath the walk would otherwise produce a
                // digest that describes neither the old nor the new content.
                if streamed != *length {
                    return Err(BridgeError::Invalid(format!(
                        "managed payload file changed size while hashing: {}",
                        path.display()
                    )));
                }
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Fold a file's bytes into `digest` without holding them in memory.
fn stream_file_into(path: &Path, digest: &mut Sha256) -> Result<u64, BridgeError> {
    let mut file = fs::File::open(path)?;
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(total);
        }
        digest.update(&buffer[..read]);
        total += read as u64;
    }
}

fn installation_id(agent: &str, version: &str, platform: &str, integrity: &str) -> String {
    let mut digest = Sha256::new();
    for value in [agent, version, platform, integrity] {
        digest.update(value.as_bytes());
        digest.update(b"\0");
    }
    format!("{:x}", digest.finalize())[..24].to_owned()
}

fn copy_payload(recipe: &ValidatedRecipe, payload: &Path) -> Result<(), BridgeError> {
    fs::create_dir_all(payload)?;
    match recipe.shape {
        PayloadShape::File => {
            let destination = payload.join(&recipe.entrypoint);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&recipe.source_path, destination)?;
        }
        PayloadShape::Directory => copy_directory(&recipe.source_path, payload)?,
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), BridgeError> {
    let mut children = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let source_path = child.path();
        let destination_path = destination.join(child.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(BridgeError::Invalid(format!(
                "managed payload source contains a symlink: {}",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_directory(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            return Err(BridgeError::Invalid(format!(
                "managed payload source contains an unsupported filesystem entry: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

/// Flush the staged tree, then its directories, in a pass of its own.
///
/// Deliberately separate from [`copy_payload`]: flushing each file immediately
/// after writing it turns every file into its own disk barrier, which measured
/// ~20x slower on a four-thousand-file payload than copying first and flushing
/// after. The fsync count is the same either way — it is the cost of the
/// durability the promotion relies on — but the ordering is not free.
fn sync_tree_files(path: &Path) -> Result<(), BridgeError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(BridgeError::Invalid(
            "managed payload staging tree cannot contain symlinks".into(),
        ));
    }
    if metadata.is_file() {
        fs::File::open(path)?.sync_all()?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(BridgeError::Invalid(
            "managed payload staging tree contains an unsupported entry".into(),
        ));
    }
    for entry in fs::read_dir(path)? {
        sync_tree_files(&entry?.path())?;
    }
    sync_directory(path)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), BridgeError> {
    let parent = path.parent().ok_or_else(|| {
        BridgeError::Invalid(format!(
            "managed payload path has no parent: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent)?;
    let encoded = serde_json::to_vec_pretty(value).map_err(|error| {
        BridgeError::Invalid(format!(
            "managed payload receipt cannot be encoded: {error}"
        ))
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&encoded)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| BridgeError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), BridgeError> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), BridgeError> {
    Ok(())
}

fn fresh_receipt(recipe: &ValidatedRecipe) -> ManagedPayloadReceipt {
    let owned_path = PathBuf::from("agents")
        .join(&recipe.agent_id)
        .join("installations")
        .join(&recipe.installation_id);
    ManagedPayloadReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        agent_id: recipe.agent_id.clone(),
        version: recipe.version.clone(),
        platform: recipe.platform.clone(),
        source: recipe.source.clone(),
        integrity_sha256: recipe.expected_sha256.clone(),
        installation_id: recipe.installation_id.clone(),
        owned_paths: vec![owned_path.clone()],
        entrypoint: owned_path.join("payload").join(&recipe.entrypoint),
        payload_shape: recipe.shape,
        payload_entrypoint: recipe.entrypoint.clone(),
        installed_at: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn file_recipe(root: &Path) -> PayloadRecipe {
        let source = root.join("agent-bin");
        fs::write(&source, b"fixture agent").unwrap();
        make_executable(&source);
        let entrypoint = PathBuf::from("bin/agent");
        PayloadRecipe {
            agent_id: "fixture-agent".into(),
            version: "1.2.3".into(),
            platform: "darwin-aarch64".into(),
            source: "fixture://agent-bin".into(),
            expected_sha256: source_digest(&source, PayloadShape::File, &entrypoint).unwrap(),
            source_path: source,
            shape: PayloadShape::File,
            entrypoint,
        }
    }

    fn directory_recipe(root: &Path) -> PayloadRecipe {
        let source = root.join("agent-tree");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/agent"), b"directory fixture agent").unwrap();
        make_executable(&source.join("bin/agent"));
        fs::write(source.join("README"), b"fixture").unwrap();
        let entrypoint = PathBuf::from("bin/agent");
        PayloadRecipe {
            agent_id: "directory-agent".into(),
            version: "4.5.6".into(),
            platform: "darwin-aarch64".into(),
            source: "fixture://agent-tree".into(),
            expected_sha256: source_digest(&source, PayloadShape::Directory, &entrypoint).unwrap(),
            source_path: source,
            shape: PayloadShape::Directory,
            entrypoint,
        }
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    #[test]
    fn recipe_rejects_unsafe_components_entrypoints_and_symlink_sources() {
        let fixture = tempfile::tempdir().unwrap();
        let base = file_recipe(fixture.path());
        for (field, value) in [
            ("agent", "../agent"),
            ("agent", "/agent"),
            ("version", ""),
            ("platform", "linux/x86_64"),
        ] {
            let mut recipe = base.clone();
            match field {
                "agent" => recipe.agent_id = value.into(),
                "version" => recipe.version = value.into(),
                "platform" => recipe.platform = value.into(),
                _ => unreachable!(),
            }
            assert!(recipe.validate().is_err(), "{field} accepted {value:?}");
        }

        let mut escaping = base.clone();
        escaping.entrypoint = PathBuf::from("../outside");
        assert!(escaping.validate().is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&base.source_path, fixture.path().join("link")).unwrap();
            let mut linked = base;
            linked.source_path = fixture.path().join("link");
            assert!(linked.validate().is_err());
        }
    }

    #[test]
    fn source_digest_is_stable_and_sensitive_to_paths_and_bytes() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("tree");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a"), b"one").unwrap();
        fs::write(root.join("nested/b"), b"two").unwrap();
        let first = source_digest(&root, PayloadShape::Directory, Path::new("a")).unwrap();
        let second = source_digest(&root, PayloadShape::Directory, Path::new("a")).unwrap();
        assert_eq!(first, second);
        fs::write(root.join("nested/b"), b"changed").unwrap();
        assert_ne!(
            first,
            source_digest(&root, PayloadShape::Directory, Path::new("a")).unwrap()
        );
    }

    #[test]
    fn file_and_directory_payloads_install_with_versioned_receipts() {
        let fixture = tempfile::tempdir().unwrap();
        let managed = fixture.path().join("managed");
        let store = ManagedPayloadStore::new(&managed);

        for recipe in [
            file_recipe(fixture.path()),
            directory_recipe(fixture.path()),
        ] {
            let outcome = store.install(&recipe).unwrap();
            let receipt = outcome.receipt();
            assert!(matches!(&outcome, InstallOutcome::Installed(_)));
            assert_eq!(receipt.schema_version, RECEIPT_SCHEMA_VERSION);
            assert_eq!(receipt.agent_id, recipe.agent_id);
            assert_eq!(receipt.version, recipe.version);
            assert_eq!(receipt.integrity_sha256, recipe.expected_sha256);
            assert!(managed.join(&receipt.entrypoint).is_file());
            let embedded = managed.join(&receipt.owned_paths[0]).join("receipt.json");
            let persisted: ManagedPayloadReceipt =
                serde_json::from_slice(&fs::read(embedded).unwrap()).unwrap();
            assert_eq!(&persisted, receipt);
        }
    }

    #[test]
    fn tampered_artifact_never_becomes_active() {
        let fixture = tempfile::tempdir().unwrap();
        let managed = fixture.path().join("managed");
        let store = ManagedPayloadStore::new(&managed);
        let recipe = file_recipe(fixture.path());
        fs::write(&recipe.source_path, b"tampered after recipe resolution").unwrap();

        assert!(store.install(&recipe).is_err());
        assert!(!managed.join("agents/fixture-agent/active.json").exists());
        assert!(!managed.join("agents/fixture-agent/installations").exists());
    }

    #[test]
    fn repeated_and_concurrent_installs_converge_to_one_owned_installation() {
        let fixture = tempfile::tempdir().unwrap();
        let store = Arc::new(ManagedPayloadStore::new(fixture.path().join("managed")));
        let recipe = Arc::new(file_recipe(fixture.path()));
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            let recipe = Arc::clone(&recipe);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                store.install(&recipe).unwrap().receipt().clone()
            }));
        }
        let receipts = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(receipts.windows(2).all(|pair| pair[0] == pair[1]));

        let installations = fs::read_dir(store.root().join("agents/fixture-agent/installations"))
            .unwrap()
            .count();
        assert_eq!(installations, 1);
        assert!(matches!(
            store.install(&recipe).unwrap(),
            InstallOutcome::AlreadyInstalled(_)
        ));
    }

    #[test]
    fn retry_recovers_a_promoted_installation_from_its_embedded_receipt() {
        let fixture = tempfile::tempdir().unwrap();
        let managed = fixture.path().join("managed");
        let store = ManagedPayloadStore::new(&managed);
        let recipe = file_recipe(fixture.path());
        store.inject_failure_after_promotion();

        assert!(store.install(&recipe).is_err());
        assert!(!managed.join("agents/fixture-agent/active.json").exists());
        let recovered = store.install(&recipe).unwrap();
        assert!(matches!(recovered, InstallOutcome::Recovered(_)));
        let active: ManagedPayloadReceipt = serde_json::from_slice(
            &fs::read(managed.join("agents/fixture-agent/active.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(&active, recovered.receipt());
    }

    #[test]
    fn status_reports_corrupt_missing_and_drifted_installations_as_repairable() {
        fn installed_case() -> (tempfile::TempDir, ManagedPayloadStore, PayloadRecipe) {
            let fixture = tempfile::tempdir().unwrap();
            let store = ManagedPayloadStore::new(fixture.path().join("managed"));
            let recipe = file_recipe(fixture.path());
            store.install(&recipe).unwrap();
            (fixture, store, recipe)
        }

        let fixture = tempfile::tempdir().unwrap();
        let absent = ManagedPayloadStore::new(fixture.path().join("managed"));
        assert_eq!(
            absent.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::NotInstalled
        );

        let (_fixture, store, _recipe) = installed_case();
        fs::write(
            store.root().join("agents/fixture-agent/active.json"),
            b"not json",
        )
        .unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::CorruptActiveReceipt
            }
        );

        let (_fixture, store, _recipe) = installed_case();
        let receipt =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        fs::remove_dir_all(store.root().join(&receipt.owned_paths[0])).unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::MissingInstallation
            }
        );

        let (_fixture, store, _recipe) = installed_case();
        let receipt =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        fs::remove_dir_all(store.root().join(&receipt.owned_paths[0]).join("payload")).unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::MissingPayload
            }
        );

        let (_fixture, store, _recipe) = installed_case();
        let receipt =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        fs::remove_file(store.root().join(&receipt.entrypoint)).unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::MissingEntrypoint
            }
        );

        let (_fixture, store, _recipe) = installed_case();
        let receipt =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        fs::write(store.root().join(&receipt.entrypoint), b"drifted").unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::IntegrityDrift
            }
        );
    }

    #[test]
    fn repair_requires_receipt_proof_and_restores_a_drifted_owned_payload() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        fs::write(store.root().join(&receipt.entrypoint), b"drifted").unwrap();

        let repaired = store.repair(&recipe).unwrap();
        assert!(matches!(repaired, RepairOutcome::Repaired(_)));
        assert!(matches!(
            store.status(&recipe.agent_id).unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));

        let current =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        fs::write(store.root().join(&current.entrypoint), b"drifted again").unwrap();
        fs::write(&recipe.source_path, b"invalid replacement").unwrap();
        assert!(store.repair(&recipe).is_err());
        assert!(store.root().join(&current.owned_paths[0]).exists());

        let unowned_fixture = tempfile::tempdir().unwrap();
        let unowned_store = ManagedPayloadStore::new(unowned_fixture.path().join("managed"));
        let unowned_recipe = file_recipe(unowned_fixture.path());
        let validated = unowned_recipe.validate().unwrap();
        let unowned_path = unowned_store
            .root()
            .join("agents/fixture-agent/installations")
            .join(&validated.installation_id);
        fs::create_dir_all(&unowned_path).unwrap();
        fs::write(unowned_path.join("sentinel"), b"keep").unwrap();
        assert!(unowned_store.repair(&unowned_recipe).is_err());
        assert!(unowned_path.join("sentinel").exists());
    }

    #[test]
    fn uninstall_removes_only_the_exact_receipt_owned_installation() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let sibling = store
            .root()
            .join("agents/fixture-agent/installations/sibling-version");
        let unrelated = store.root().join("unrelated");
        let external = fixture.path().join("external-agent");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("keep"), b"keep").unwrap();
        fs::write(&unrelated, b"keep").unwrap();
        fs::write(&external, b"keep").unwrap();

        assert_eq!(
            store.uninstall("fixture-agent").unwrap(),
            UninstallOutcome::Uninstalled
        );
        assert!(!store.root().join(&receipt.owned_paths[0]).exists());
        assert!(!store
            .root()
            .join("agents/fixture-agent/active.json")
            .exists());
        assert!(sibling.join("keep").exists());
        assert!(unrelated.exists());
        assert!(external.exists());
    }

    #[test]
    fn uninstall_fails_closed_for_corrupt_or_forged_receipts() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let active_path = store.root().join("agents/fixture-agent/active.json");
        let owned_path = store.root().join(&receipt.owned_paths[0]);

        fs::write(&active_path, b"broken").unwrap();
        assert!(store.uninstall("fixture-agent").is_err());
        assert!(owned_path.exists());

        for forged_path in [
            PathBuf::from("../outside"),
            PathBuf::from("/tmp/outside"),
            PathBuf::from("agents/fixture-agent/installations/wrong-id"),
        ] {
            let mut forged = receipt.clone();
            forged.owned_paths = vec![forged_path];
            write_json_atomic(&active_path, &forged).unwrap();
            assert!(store.uninstall("fixture-agent").is_err());
            assert!(owned_path.exists());
        }

        write_json_atomic(&active_path, &receipt).unwrap();
        let mut mismatched = receipt.clone();
        mismatched.source = "fixture://forged".into();
        write_json_atomic(&owned_path.join("receipt.json"), &mismatched).unwrap();
        assert!(store.uninstall("fixture-agent").is_err());
        assert!(owned_path.exists());

        #[cfg(unix)]
        {
            write_json_atomic(&owned_path.join("receipt.json"), &receipt).unwrap();
            let real_installations = store.root().join("real-installations");
            fs::rename(
                store.root().join("agents/fixture-agent/installations"),
                &real_installations,
            )
            .unwrap();
            std::os::unix::fs::symlink(
                &real_installations,
                store.root().join("agents/fixture-agent/installations"),
            )
            .unwrap();
            assert!(store.uninstall("fixture-agent").is_err());
            assert!(real_installations.exists());
        }
    }

    #[test]
    fn repeated_uninstall_converges_and_external_detection_never_claims_ownership() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let external = fixture.path().join("external-agent");
        fs::write(&external, b"external").unwrap();
        make_executable(&external);
        store.install(&recipe).unwrap();

        let inspection = inspect_external_runtime(&external);
        assert!(inspection.available);
        assert_eq!(inspection.candidate, external);
        assert_eq!(
            store.uninstall("fixture-agent").unwrap(),
            UninstallOutcome::Uninstalled
        );
        assert_eq!(
            store.uninstall("fixture-agent").unwrap(),
            UninstallOutcome::AlreadyAbsent
        );
        assert!(external.exists());
        assert!(!external.with_extension("json").exists());

        store.install(&recipe).unwrap();
        assert!(matches!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));
    }

    #[test]
    fn different_agents_have_independent_serialization_and_receipts() {
        let fixture = tempfile::tempdir().unwrap();
        let store = Arc::new(ManagedPayloadStore::new(fixture.path().join("managed")));
        let first = file_recipe(fixture.path());
        let mut second = directory_recipe(fixture.path());
        second.agent_id = "second-agent".into();
        let first_thread = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.install(&first).unwrap().receipt().clone())
        };
        let second_thread = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.install(&second).unwrap().receipt().clone())
        };
        let first_receipt = first_thread.join().unwrap();
        let second_receipt = second_thread.join().unwrap();
        assert_ne!(first_receipt.agent_id, second_receipt.agent_id);
        assert_ne!(first_receipt.owned_paths, second_receipt.owned_paths);
        assert!(store
            .root()
            .join("agents/fixture-agent/active.json")
            .exists());
        assert!(store
            .root()
            .join("agents/second-agent/active.json")
            .exists());
    }

    #[test]
    fn concurrent_install_and_uninstall_converge_without_cross_owned_deletion() {
        let fixture = tempfile::tempdir().unwrap();
        let store = Arc::new(ManagedPayloadStore::new(fixture.path().join("managed")));
        let recipe = Arc::new(file_recipe(fixture.path()));
        store.install(&recipe).unwrap();
        let sibling = store
            .root()
            .join("agents/fixture-agent/installations/sibling-version");
        let external = fixture.path().join("external-agent");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("keep"), b"keep").unwrap();
        fs::write(&external, b"keep").unwrap();
        let barrier = Arc::new(Barrier::new(2));

        let install_thread = {
            let store = Arc::clone(&store);
            let recipe = Arc::clone(&recipe);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.install(&recipe)
            })
        };
        let uninstall_thread = {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.uninstall("fixture-agent")
            })
        };
        install_thread.join().unwrap().unwrap();
        uninstall_thread.join().unwrap().unwrap();

        match store.status("fixture-agent").unwrap() {
            ManagedPayloadStatus::Installed { receipt, .. } => {
                assert!(store.root().join(&receipt.owned_paths[0]).is_dir());
            }
            ManagedPayloadStatus::NotInstalled => {
                assert!(!store
                    .root()
                    .join("agents/fixture-agent/active.json")
                    .exists());
            }
            status => panic!("concurrent lifecycle left partial state: {status:?}"),
        }
        assert!(sibling.join("keep").exists());
        assert!(external.exists());
    }

    /// Provenance is not identity. Relabelling a byte-identical artifact used
    /// to map onto the same installation and then fail its own ownership check,
    /// leaving install and repair permanently erroring while status still said
    /// `Installed`.
    #[test]
    fn relabelled_provenance_converges_instead_of_wedging_the_agent() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let mirrored = file_recipe(fixture.path());
        let first = store.install(&mirrored).unwrap().receipt().clone();

        let mut vendored = mirrored.clone();
        vendored.source = "https://cdn.example/agent-1.2.3".into();
        assert_eq!(
            mirrored.validate().unwrap().installation_id,
            vendored.validate().unwrap().installation_id,
            "identity must not depend on the provenance label"
        );

        assert!(matches!(
            store.install(&vendored).unwrap(),
            InstallOutcome::AlreadyInstalled(_)
        ));
        assert!(matches!(
            store.repair(&vendored).unwrap(),
            RepairOutcome::AlreadyHealthy(_)
        ));
        assert!(matches!(
            store.status(&vendored.agent_id).unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));

        // The receipt keeps where the installed bytes actually came from.
        let active =
            read_receipt_if_valid(&store.root().join("agents/fixture-agent/active.json")).unwrap();
        assert_eq!(active.source, first.source);
        assert_eq!(active.source, "fixture://agent-bin");
    }

    /// The digest describes the whole payload tree for both shapes, so nothing
    /// planted beside a single-file entrypoint is invisible any more.
    #[test]
    fn integrity_covers_the_whole_payload_tree_for_both_shapes() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let payload = store.root().join(&receipt.owned_paths[0]).join("payload");

        // Hashing the source and hashing the installed tree agree by
        // construction, including the directories the entrypoint implies.
        assert_eq!(
            payload_tree_digest(&payload).unwrap(),
            receipt.integrity_sha256
        );

        fs::write(payload.join("libevil.dylib"), b"planted").unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::IntegrityDrift
            },
            "a file planted beside a file-shaped entrypoint is drift"
        );

        // Directories are hashed too, so losing an empty one is drift rather
        // than an invisible change.
        let tree_fixture = tempfile::tempdir().unwrap();
        let mut tree = directory_recipe(tree_fixture.path());
        fs::create_dir_all(tree.source_path.join("plugins")).unwrap();
        tree.expected_sha256 =
            source_digest(&tree.source_path, PayloadShape::Directory, &tree.entrypoint).unwrap();
        let tree_store = ManagedPayloadStore::new(tree_fixture.path().join("managed"));
        let tree_receipt = tree_store.install(&tree).unwrap().receipt().clone();
        fs::remove_dir(
            tree_store
                .root()
                .join(&tree_receipt.owned_paths[0])
                .join("payload/plugins"),
        )
        .unwrap();
        assert_eq!(
            tree_store.status(&tree.agent_id).unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::IntegrityDrift
            }
        );
    }

    /// The digest is a wire format the moment a recipe ships an expected value,
    /// so pin it: `/`-joined UTF-8 paths, directories included, no lossy
    /// conversion that could let two different trees collide.
    #[test]
    fn digest_format_is_pinned_and_platform_independent() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("tree");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("bin/agent"), b"agent").unwrap();
        fs::write(root.join("README"), b"readme").unwrap();
        assert_eq!(
            source_digest(&root, PayloadShape::Directory, Path::new("bin/agent")).unwrap(),
            "b64fedab1a3c93e8c23441041c8d478a537ee71c3f04547763b94c6bb720612c",
            "changing the digest format invalidates every published recipe"
        );

        assert_eq!(
            canonical_relative("entrypoint", Path::new("bin/agent")).unwrap(),
            "bin/agent"
        );
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            // Two distinct non-UTF-8 names both became U+FFFD under
            // `to_string_lossy`, which let different trees share a digest.
            assert!(
                canonical_relative("artifact path", Path::new(OsStr::from_bytes(b"bin/\xff")))
                    .is_err()
            );
        }
    }

    /// Mode is outside the digest by design, so install asserts the entrypoint
    /// is runnable and status reports it when that stops being true.
    #[test]
    #[cfg(unix)]
    fn entrypoint_executability_is_enforced_and_reported() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        // Arrive over a transport that dropped the permission bits.
        fs::set_permissions(&recipe.source_path, fs::Permissions::from_mode(0o644)).unwrap();

        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let entrypoint = store.root().join(&receipt.entrypoint);
        assert!(
            fs::metadata(&entrypoint).unwrap().permissions().mode() & 0o111 != 0,
            "install must produce a runnable entrypoint"
        );

        fs::set_permissions(&entrypoint, fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::EntrypointNotExecutable
            },
            "an unrunnable install is not Installed"
        );

        assert!(matches!(
            store.repair(&recipe).unwrap(),
            RepairOutcome::Repaired(_)
        ));
        assert!(matches!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));
    }

    /// Superseded versions are Bridge-owned and unreachable once the active
    /// receipt moves on, so both install and uninstall reclaim them — and
    /// neither touches a directory whose ownership is unproven.
    #[test]
    fn superseded_installations_are_reclaimed_but_unproven_ones_are_not() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let installations = store.root().join("agents/fixture-agent/installations");
        let v1 = file_recipe(fixture.path());
        let old = store.install(&v1).unwrap().receipt().clone();

        let v2_source = fixture.path().join("agent-bin-v2");
        fs::write(&v2_source, b"fixture agent v2").unwrap();
        make_executable(&v2_source);
        let mut v2 = v1.clone();
        v2.version = "2.0.0".into();
        v2.source_path = v2_source;
        v2.expected_sha256 =
            source_digest(&v2.source_path, PayloadShape::File, &v2.entrypoint).unwrap();

        let unproven = installations.join("hand-made");
        fs::create_dir_all(&unproven).unwrap();
        fs::write(unproven.join("keep"), b"keep").unwrap();

        let new = store.install(&v2).unwrap().receipt().clone();
        assert!(!store.root().join(&old.owned_paths[0]).exists(), "upgrade reclaims the old version");
        assert!(store.root().join(&new.owned_paths[0]).is_dir());
        assert!(unproven.join("keep").exists(), "unproven content is never pruned");

        // A receipt-owned installation that nothing points at is still ours.
        let orphan_recipe = v1.validate().unwrap();
        let orphan_root = installations.join(&orphan_recipe.installation_id);
        fs::create_dir_all(orphan_root.join("payload")).unwrap();
        write_json_atomic(
            &orphan_root.join("receipt.json"),
            &fresh_receipt(&orphan_recipe),
        )
        .unwrap();

        store.uninstall("fixture-agent").unwrap();
        assert!(!store.root().join(&new.owned_paths[0]).exists());
        assert!(!orphan_root.exists(), "uninstall reclaims owned orphans");
        assert!(unproven.join("keep").exists());
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::NotInstalled
        );
    }

    /// A process killed mid-install leaves a partial copy behind. The next
    /// install for that agent discards it, and cannot reach another agent's.
    #[test]
    fn staging_orphans_are_reaped_without_crossing_agents() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        store.install(&recipe).unwrap();

        let ours = store
            .root()
            .join(".staging/fixture-agent/deadbeefcrash");
        // A prefix-matching agent id must not be collateral: staging is keyed
        // by directory, not by name prefix.
        let theirs = store
            .root()
            .join(".staging/fixture-agent-sidecar/inflight");
        fs::create_dir_all(&ours).unwrap();
        fs::write(ours.join("payload-fragment"), b"partial").unwrap();
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("payload-fragment"), b"partial").unwrap();

        store.uninstall("fixture-agent").unwrap();
        store.install(&recipe).unwrap();
        assert!(!ours.exists(), "our own staging orphan is reaped");
        assert!(theirs.join("payload-fragment").exists());
    }

    /// The agent lock guards no data, so a panic under it must not brick every
    /// later lifecycle call for the rest of the process.
    #[test]
    fn lifecycle_survives_a_poisoned_agent_lock() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("managed");
        let store = ManagedPayloadStore::new(&root);
        let recipe = file_recipe(fixture.path());
        store.install(&recipe).unwrap();

        let lock = agent_lock(&root, "fixture-agent");
        assert!(std::thread::spawn(move || {
            let _held = lock.lock().unwrap();
            panic!("poison the agent lock");
        })
        .join()
        .is_err());

        assert!(matches!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));
        assert!(matches!(
            store.install(&recipe).unwrap(),
            InstallOutcome::AlreadyInstalled(_)
        ));
        assert!(matches!(
            store.repair(&recipe).unwrap(),
            RepairOutcome::AlreadyHealthy(_)
        ));
        assert_eq!(
            store.uninstall("fixture-agent").unwrap(),
            UninstallOutcome::Uninstalled
        );
    }

    /// Repairing a corrupt embedded receipt means replacing the directory it
    /// was supposed to describe. That is allowed only while the directory still
    /// looks like a Bridge installation — never when it holds foreign content.
    #[test]
    fn repair_refuses_to_replace_a_directory_holding_foreign_content() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let installation = store.root().join(&receipt.owned_paths[0]);

        fs::write(installation.join("receipt.json"), b"corrupt").unwrap();
        fs::write(installation.join("user-sentinel"), b"do not delete").unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::CorruptEmbeddedReceipt
            }
        );

        let refused = store.repair(&recipe).unwrap_err().to_string();
        assert!(
            refused.contains("shape of a Bridge installation"),
            "unexpected error: {refused}"
        );
        assert!(installation.join("user-sentinel").exists());

        // Without the foreign file, a corrupt receipt is still recoverable.
        fs::remove_file(installation.join("user-sentinel")).unwrap();
        assert!(matches!(
            store.repair(&recipe).unwrap(),
            RepairOutcome::Repaired(_)
        ));
        assert!(matches!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));
    }

    /// Install verifies the bytes it actually stored, not the bytes it was
    /// promised — the one check that survives a source swapped mid-copy.
    #[test]
    fn install_verifies_the_stored_representation() {
        let fixture = tempfile::tempdir().unwrap();
        let managed = fixture.path().join("managed");
        let store = ManagedPayloadStore::new(&managed);
        let recipe = file_recipe(fixture.path());
        fs::write(&recipe.source_path, b"tampered after recipe resolution").unwrap();

        let error = store.install(&recipe).unwrap_err().to_string();
        assert!(
            error.contains("staged integrity mismatch"),
            "unexpected error: {error}"
        );
        assert!(!managed.join("agents/fixture-agent/active.json").exists());
        assert!(!managed.join("agents/fixture-agent/installations").exists());
        // Staging is discarded rather than left for the next install to find.
        assert!(
            fs::read_dir(managed.join(".staging/fixture-agent"))
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0
        );
    }

    /// Receipt reads are bounded, and a receipt that does not name exactly one
    /// owned path is a repair reason rather than an index panic.
    #[test]
    fn receipts_are_bounded_and_owned_paths_are_checked() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let recipe = file_recipe(fixture.path());
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        let active_path = store.root().join("agents/fixture-agent/active.json");

        let mut oversized = serde_json::to_vec(&receipt).unwrap();
        oversized.extend(std::iter::repeat_n(b' ', MAX_RECEIPT_BYTES as usize + 1));
        fs::write(&active_path, &oversized).unwrap();
        assert_eq!(
            store.status("fixture-agent").unwrap(),
            ManagedPayloadStatus::Repairable {
                reason: RepairReason::CorruptActiveReceipt
            }
        );
        assert!(store.uninstall("fixture-agent").is_err());

        for owned in [vec![], vec![receipt.owned_paths[0].clone(); 2]] {
            let mut forged = receipt.clone();
            forged.owned_paths = owned;
            write_json_atomic(&active_path, &forged).unwrap();
            assert_eq!(
                store.status("fixture-agent").unwrap(),
                ManagedPayloadStatus::Repairable {
                    reason: RepairReason::ActiveReceiptMismatch
                }
            );
            assert!(store.uninstall("fixture-agent").is_err());
        }
        assert!(store.root().join(&receipt.owned_paths[0]).is_dir());
    }
}
