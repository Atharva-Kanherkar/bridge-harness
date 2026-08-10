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
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

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
}
