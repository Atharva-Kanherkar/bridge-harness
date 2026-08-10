//! Receipt-owned payload storage for Bridge-managed agent runtimes.
//!
//! This module deliberately has no network, RPC, UI, or vendor-authentication
//! concerns. Callers provide an already resolved artifact plus its expected
//! integrity. Bridge stages and owns only what it can prove through receipts.

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
    pub installed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed(ManagedPayloadReceipt),
    AlreadyInstalled(ManagedPayloadReceipt),
    Recovered(ManagedPayloadReceipt),
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
        let lock = agent_lock(&self.root, &recipe.agent_id)?;
        let _guard = lock
            .lock()
            .map_err(|_| BridgeError::Invalid("managed payload agent lock was poisoned".into()))?;
        self.install_locked(&recipe)
    }

    fn install_locked(&self, recipe: &ValidatedRecipe) -> Result<InstallOutcome, BridgeError> {
        ensure_root_is_not_symlink(&self.root)?;
        let actual_source_digest =
            source_digest(&recipe.source_path, recipe.shape, &recipe.entrypoint)?;
        if actual_source_digest != recipe.expected_sha256 {
            return Err(BridgeError::Invalid(format!(
                "managed payload source integrity mismatch: expected {}, got {actual_source_digest}",
                recipe.expected_sha256
            )));
        }

        let agent_root = self.root.join("agents").join(&recipe.agent_id);
        let installations_root = agent_root.join("installations");
        let installation_root = installations_root.join(&recipe.installation_id);
        let active_path = agent_root.join("active.json");

        if installation_root.exists() {
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

        fs::create_dir_all(&installations_root)?;
        let staging_root = self.root.join(".staging");
        fs::create_dir_all(&staging_root)?;
        let staging_path =
            staging_root.join(format!("{}-{}", recipe.agent_id, Uuid::new_v4().simple()));
        fs::create_dir(&staging_path)?;
        let mut staging = StagingGuard {
            path: staging_path.clone(),
            armed: true,
        };
        let staged_payload = staging_path.join("payload");
        copy_payload(recipe, &staged_payload)?;
        sync_tree_files(&staged_payload)?;
        let staged_digest = installed_payload_digest(&staged_payload, recipe)?;
        if staged_digest != recipe.expected_sha256 {
            return Err(BridgeError::Invalid(format!(
                "managed payload staged integrity mismatch: expected {}, got {staged_digest}",
                recipe.expected_sha256
            )));
        }
        let staged_entrypoint = staged_payload.join(&recipe.entrypoint);
        if !staged_entrypoint.is_file() {
            return Err(BridgeError::Invalid(format!(
                "managed payload entrypoint is not a file: {}",
                recipe.entrypoint.display()
            )));
        }

        let receipt = fresh_receipt(recipe);
        write_json_atomic(&staging_path.join("receipt.json"), &receipt)?;
        sync_directory(&staging_path)?;
        match fs::rename(&staging_path, &installation_root) {
            Ok(()) => {
                staging.armed = false;
                sync_directory(&installations_root)?;
            }
            Err(_error) if installation_root.exists() => {
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
        Ok(InstallOutcome::Installed(receipt))
    }

    #[cfg(test)]
    fn inject_failure_after_promotion(&self) {
        self.fail_after_promotion
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn agent_lock(root: &Path, agent_id: &str) -> Result<Arc<Mutex<()>>, BridgeError> {
    let key = root.join("agents").join(agent_id);
    let mut locks = AGENT_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| BridgeError::Invalid("managed payload lock registry was poisoned".into()))?;
    Ok(locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
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

fn read_receipt_if_valid(path: &Path) -> Option<ManagedPayloadReceipt> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
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
    let receipt: ManagedPayloadReceipt = serde_json::from_slice(&fs::read(&receipt_path)?)
        .map_err(|error| {
            BridgeError::Invalid(format!("managed payload receipt is corrupt: {error}"))
        })?;
    let expected = fresh_receipt(recipe);
    if !receipt_matches_recipe(&receipt, &expected) {
        return Err(BridgeError::Invalid(
            "managed payload receipt does not prove ownership of the requested installation".into(),
        ));
    }
    let payload = installation_root.join("payload");
    let digest = installed_payload_digest(&payload, recipe)?;
    if digest != recipe.expected_sha256 {
        return Err(BridgeError::Invalid(
            "managed payload installation exists but its integrity has drifted".into(),
        ));
    }
    if !root.join(&receipt.entrypoint).is_file() {
        return Err(BridgeError::Invalid(
            "managed payload installation exists but its entrypoint is missing".into(),
        ));
    }
    Ok(receipt)
}

fn receipt_matches_recipe(
    receipt: &ManagedPayloadReceipt,
    expected: &ManagedPayloadReceipt,
) -> bool {
    receipt.schema_version == RECEIPT_SCHEMA_VERSION
        && receipt.agent_id == expected.agent_id
        && receipt.version == expected.version
        && receipt.platform == expected.platform
        && receipt.source == expected.source
        && receipt.integrity_sha256 == expected.integrity_sha256
        && receipt.installation_id == expected.installation_id
        && receipt.owned_paths == expected.owned_paths
        && receipt.entrypoint == expected.entrypoint
}

fn installed_payload_digest(
    payload_root: &Path,
    recipe: &ValidatedRecipe,
) -> Result<String, BridgeError> {
    match recipe.shape {
        PayloadShape::File => source_digest(
            &payload_root.join(&recipe.entrypoint),
            PayloadShape::File,
            &recipe.entrypoint,
        ),
        PayloadShape::Directory => {
            source_digest(payload_root, PayloadShape::Directory, &recipe.entrypoint)
        }
    }
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

pub fn source_digest(
    source: &Path,
    shape: PayloadShape,
    entrypoint: &Path,
) -> Result<String, BridgeError> {
    validate_relative_path("entrypoint", entrypoint)?;
    validate_source_root(source, shape)?;
    let mut entries = Vec::new();
    match shape {
        PayloadShape::File => hash_file_entry(source, entrypoint, &mut entries)?,
        PayloadShape::Directory => collect_source_entries(source, source, &mut entries)?,
    }
    Ok(hash_entries(entries))
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

fn collect_source_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<(PathBuf, Vec<u8>)>,
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
        if metadata.is_dir() {
            collect_source_entries(root, &path, entries)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).map_err(|_| {
                BridgeError::Invalid("managed payload source escaped its root".into())
            })?;
            hash_file_entry(&path, relative, entries)?;
        } else {
            return Err(BridgeError::Invalid(format!(
                "managed payload source contains an unsupported filesystem entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn hash_file_entry(
    path: &Path,
    relative: &Path,
    entries: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), BridgeError> {
    validate_relative_path("artifact path", relative)?;
    let mut bytes = Vec::new();
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    entries.push((relative.to_path_buf(), bytes));
    Ok(())
}

fn hash_entries(mut entries: Vec<(PathBuf, Vec<u8>)>) -> String {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, bytes) in entries {
        let path = path.to_string_lossy();
        digest.update(b"file\0");
        digest.update(path.as_bytes());
        digest.update(b"\0");
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    format!("{:x}", digest.finalize())
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
}
