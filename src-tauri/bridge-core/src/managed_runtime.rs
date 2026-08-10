//! Official-source resolution for Bridge-managed agent runtimes.
//!
//! Bridge is not a distributor. Payload bytes always come from the vendor's own
//! source; what Bridge ships is the pinned version, the expected integrity where
//! the publisher provides one, and the resolution order below.
//!
//! # Two integrity models
//!
//! The three runtimes are not the same shape, and pretending otherwise would
//! mean claiming a guarantee one of them cannot give:
//!
//!   * A [`RuntimeSource::ReleaseArtifact`] is a single published file, so the
//!     recipe carries a publisher-pinned SHA-256 and a mismatch is fatal before
//!     anything is promoted.
//!   * A [`RuntimeSource::NpmClosure`] is a dependency *closure*. An `npm`
//!     install tree is not byte-reproducible across machines — the Claude SDK
//!     alone pulls platform-specific binaries — so there is no honest constant to
//!     pin it against. Its supply-chain guarantee comes from npm verifying every
//!     tarball against the per-package integrity in a committed lockfile, and the
//!     #174 tree digest is computed from the installed result. That digest still
//!     does what it was built for: proving ownership and catching later drift.
//!
//! # Resolution order
//!
//! Explicit user configuration, then a Bridge-managed receipt-bound payload, then
//! a copy bundled with the app, then the system PATH. A runtime found on PATH is
//! never claimed as Bridge-managed: it resolves as
//! [`RuntimeResolution::External`], stays usable, and is Bridge's to launch but
//! never to remove.

use crate::managed_payload::{
    inspect_external_runtime, ManagedPayloadStatus, ManagedPayloadStore, PayloadShape,
};
use crate::BridgeError;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};

/// Read granularity while streaming bytes through a hasher.
const HASH_CHUNK_BYTES: usize = 128 * 1024;
/// Ceiling on one archive entry. A single agent runtime file above this is not
/// something Bridge should be unpacking without a deliberate change.
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Ceiling on an entire extracted archive.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Ceiling on entry count, so a pathological archive cannot exhaust inodes.
pub const MAX_ENTRIES: usize = 200_000;

/// What kind of file a release artifact is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A single executable, installed as the entrypoint itself.
    RawBinary,
    /// A gzipped tarball, extracted with every entry validated first.
    TarGz,
}

/// Where a managed payload's bytes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeSource {
    /// A published release file with a publisher-pinned digest.
    ReleaseArtifact {
        url: String,
        sha256: String,
        kind: ArtifactKind,
        /// Path of the executable inside the extracted archive, or the file name
        /// to give a raw binary.
        entrypoint: PathBuf,
    },
    /// An npm dependency closure pinned by an exact version and a lockfile.
    NpmClosure {
        package: String,
        version: String,
        /// Contents of the `package.json` Bridge writes into staging.
        manifest: &'static str,
        /// Contents of the `package-lock.json` that pins every tarball's
        /// integrity. This is the supply-chain guarantee for this source kind.
        lockfile: &'static str,
        /// Module entry inside the installed tree, relative to the payload root.
        entrypoint: PathBuf,
    },
}

impl RuntimeSource {
    /// The payload shape this source produces once staged.
    pub const fn shape(&self) -> PayloadShape {
        match self {
            Self::ReleaseArtifact {
                kind: ArtifactKind::RawBinary,
                ..
            } => PayloadShape::File,
            Self::ReleaseArtifact { .. } | Self::NpmClosure { .. } => PayloadShape::Directory,
        }
    }

    pub fn entrypoint(&self) -> &Path {
        match self {
            Self::ReleaseArtifact { entrypoint, .. } | Self::NpmClosure { entrypoint, .. } => {
                entrypoint
            }
        }
    }

    /// Reject anything unpinned or not fetchable from an official source.
    ///
    /// A floating npm range is refused outright: "install the same official
    /// dependency at a pinned version" is not satisfiable by a range, and a range
    /// would make the installed tree depend on when the user clicked install.
    pub fn validate(&self) -> Result<(), BridgeError> {
        validate_relative_entrypoint(self.entrypoint())?;
        match self {
            Self::ReleaseArtifact {
                url, sha256, kind, ..
            } => {
                if !url.starts_with("https://") {
                    return Err(BridgeError::Invalid(format!(
                        "managed runtime source must be fetched over https: {url}"
                    )));
                }
                if sha256.trim().len() != 64
                    || !sha256.trim().bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(BridgeError::Invalid(
                        "managed runtime release artifact must pin a 64-character SHA-256".into(),
                    ));
                }
                if *kind == ArtifactKind::TarGz
                    && !(url.ends_with(".tar.gz") || url.ends_with(".tgz"))
                {
                    return Err(BridgeError::Invalid(format!(
                        "managed runtime archive must be a .tar.gz or .tgz: {url}"
                    )));
                }
                Ok(())
            }
            Self::NpmClosure {
                package,
                version,
                manifest,
                lockfile,
                ..
            } => {
                if package.trim().is_empty() {
                    return Err(BridgeError::Invalid(
                        "managed runtime npm package must be named".into(),
                    ));
                }
                if !version_is_exact(version) {
                    return Err(BridgeError::Invalid(format!(
                        "managed runtime npm version must be exact, not a range: {version}"
                    )));
                }
                if manifest.trim().is_empty() || lockfile.trim().is_empty() {
                    return Err(BridgeError::Invalid(
                        "managed runtime npm closure must ship a manifest and a lockfile so every \
                         tarball's integrity is pinned"
                            .into(),
                    ));
                }
                if !lockfile.contains("\"integrity\"") {
                    return Err(BridgeError::Invalid(
                        "managed runtime lockfile carries no integrity hashes, so it pins nothing"
                            .into(),
                    ));
                }
                Ok(())
            }
        }
    }
}

/// An exact npm version: digits and dots, with optional prerelease, and none of
/// the range operators.
fn version_is_exact(version: &str) -> bool {
    let version = version.trim();
    !version.is_empty()
        && version
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_digit())
        && !version
            .bytes()
            .any(|byte| matches!(byte, b'^' | b'~' | b'*' | b'>' | b'<' | b'=' | b' ' | b'|'))
        && version.split('.').count() >= 3
}

fn validate_relative_entrypoint(entrypoint: &Path) -> Result<(), BridgeError> {
    if entrypoint.as_os_str().is_empty() || entrypoint.is_absolute() {
        return Err(BridgeError::Invalid(
            "managed runtime entrypoint must be a non-empty relative path".into(),
        ));
    }
    if entrypoint
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BridgeError::Invalid(
            "managed runtime entrypoint cannot traverse".into(),
        ));
    }
    Ok(())
}

/// Which stage of preparation failed, so a caller can say something better than
/// "install failed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareStage {
    Fetch,
    Integrity,
    Extract,
    Install,
}

impl PrepareStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Integrity => "integrity",
            Self::Extract => "extract",
            Self::Install => "install",
        }
    }
}

/// Fetches vendor bytes. Injected so no test reaches the network.
pub trait ArtifactFetcher: Send + Sync {
    /// Stream `url` into `destination`. Streaming rather than returning a buffer
    /// because a runtime archive is hundreds of megabytes.
    fn fetch_to(&self, url: &str, destination: &Path) -> Result<(), BridgeError>;
}

/// The default fetcher: a blocking HTTPS GET streamed to disk.
pub struct HttpsArtifactFetcher;

impl ArtifactFetcher for HttpsArtifactFetcher {
    fn fetch_to(&self, url: &str, destination: &Path) -> Result<(), BridgeError> {
        let mut response = reqwest::blocking::Client::builder()
            .build()
            .and_then(|client| client.get(url).send())
            .map_err(|error| {
                BridgeError::Invalid(format!("managed runtime fetch failed for {url}: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(BridgeError::Invalid(format!(
                "managed runtime fetch for {url} returned {}",
                response.status()
            )));
        }
        let mut file = fs::File::create(destination)?;
        response
            .copy_to(&mut file)
            .map_err(|error| BridgeError::Invalid(format!("managed runtime fetch failed: {error}")))?;
        Ok(())
    }
}

/// A staged payload ready to hand to [`crate::managed_payload`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedRuntime {
    /// Directory (or file) to install from.
    pub source_path: PathBuf,
    pub shape: PayloadShape,
    /// Digest of the staged tree, computed the same way the engine will.
    pub sha256: String,
    pub entrypoint: PathBuf,
}

/// Fetch, verify, and unpack a source into `staging`, returning what the payload
/// engine needs.
///
/// Nothing is written outside `staging`, and a failure at any stage names that
/// stage rather than collapsing into a generic error.
pub fn prepare(
    source: &RuntimeSource,
    staging: &Path,
    fetcher: &dyn ArtifactFetcher,
) -> Result<StagedRuntime, (PrepareStage, BridgeError)> {
    source
        .validate()
        .map_err(|error| (PrepareStage::Integrity, error))?;
    fs::create_dir_all(staging).map_err(|error| (PrepareStage::Extract, error.into()))?;

    match source {
        RuntimeSource::ReleaseArtifact {
            url,
            sha256,
            kind,
            entrypoint,
        } => {
            let download = staging.join("download.part");
            fetcher
                .fetch_to(url, &download)
                .map_err(|error| (PrepareStage::Fetch, error))?;

            let actual = file_digest(&download).map_err(|error| (PrepareStage::Integrity, error))?;
            if !actual.eq_ignore_ascii_case(sha256.trim()) {
                let _ = fs::remove_file(&download);
                return Err((
                    PrepareStage::Integrity,
                    BridgeError::Invalid(format!(
                        "managed runtime artifact integrity mismatch for {url}: expected {}, got {actual}",
                        sha256.trim()
                    )),
                ));
            }

            match kind {
                ArtifactKind::RawBinary => {
                    let payload = staging.join("payload");
                    fs::create_dir_all(&payload)
                        .map_err(|error| (PrepareStage::Extract, error.into()))?;
                    let destination = payload.join(entrypoint);
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|error| (PrepareStage::Extract, error.into()))?;
                    }
                    fs::rename(&download, &destination)
                        .map_err(|error| (PrepareStage::Extract, error.into()))?;
                    Ok(StagedRuntime {
                        source_path: destination,
                        shape: PayloadShape::File,
                        sha256: actual,
                        entrypoint: entrypoint.clone(),
                    })
                }
                ArtifactKind::TarGz => {
                    let unpacked = staging.join("unpacked");
                    extract_tar_gz(&download, &unpacked)
                        .map_err(|error| (PrepareStage::Extract, error))?;
                    let _ = fs::remove_file(&download);
                    ensure_entrypoint_present(&unpacked, entrypoint)
                        .map_err(|error| (PrepareStage::Extract, error))?;
                    let digest = tree_digest(&unpacked, entrypoint)
                        .map_err(|error| (PrepareStage::Integrity, error))?;
                    Ok(StagedRuntime {
                        source_path: unpacked,
                        shape: PayloadShape::Directory,
                        sha256: digest,
                        entrypoint: entrypoint.clone(),
                    })
                }
            }
        }
        RuntimeSource::NpmClosure {
            package,
            version,
            manifest,
            lockfile,
            entrypoint,
        } => {
            let tree = staging.join("closure");
            fs::create_dir_all(&tree).map_err(|error| (PrepareStage::Extract, error.into()))?;
            fs::write(tree.join("package.json"), manifest)
                .map_err(|error| (PrepareStage::Extract, error.into()))?;
            fs::write(tree.join("package-lock.json"), lockfile)
                .map_err(|error| (PrepareStage::Extract, error.into()))?;

            // `npm ci` installs exactly the lockfile, verifying each tarball
            // against its recorded integrity. That is the supply-chain check for
            // this source kind; the tree digest below is drift detection.
            let output = Command::new("npm")
                .args(["ci", "--omit=dev", "--no-audit", "--no-fund"])
                .current_dir(&tree)
                .output()
                .map_err(|error| {
                    (
                        PrepareStage::Install,
                        BridgeError::Invalid(format!(
                            "managed runtime needs npm to install {package}@{version}: {error}"
                        )),
                    )
                })?;
            if !output.status.success() {
                return Err((
                    PrepareStage::Install,
                    BridgeError::Invalid(format!(
                        "npm ci failed for {package}@{version}: {}",
                        crate::secret_interception::sanitize(&String::from_utf8_lossy(
                            &output.stderr
                        ))
                        .text
                    )),
                ));
            }
            ensure_entrypoint_present(&tree, entrypoint)
                .map_err(|error| (PrepareStage::Install, error))?;
            let digest = tree_digest(&tree, entrypoint)
                .map_err(|error| (PrepareStage::Integrity, error))?;
            Ok(StagedRuntime {
                source_path: tree,
                shape: PayloadShape::Directory,
                sha256: digest,
                entrypoint: entrypoint.clone(),
            })
        }
    }
}

fn ensure_entrypoint_present(root: &Path, entrypoint: &Path) -> Result<(), BridgeError> {
    if root.join(entrypoint).is_file() {
        Ok(())
    } else {
        Err(BridgeError::Invalid(format!(
            "managed runtime entrypoint {} is missing from the staged payload",
            entrypoint.display()
        )))
    }
}

fn tree_digest(root: &Path, entrypoint: &Path) -> Result<String, BridgeError> {
    crate::managed_payload::source_digest(root, PayloadShape::Directory, entrypoint)
}

fn file_digest(path: &Path) -> Result<String, BridgeError> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(format!("{:x}", digest.finalize()));
        }
        digest.update(&buffer[..read]);
    }
}

/// Extract a gzipped tarball, validating every entry *before* writing it.
///
/// Validating after the fact would be too late: a tarbomb with `../` entries or
/// an absolute path has already written outside the destination by then. Only
/// regular files and directories are accepted — a symlink or hardlink entry is a
/// way to point Bridge-owned storage at something Bridge does not own, and the
/// payload engine would reject the result anyway.
pub fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<(), BridgeError> {
    use tar::EntryType;

    fs::create_dir_all(destination)?;
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut total = 0u64;
    let mut count = 0usize;

    for entry in tar.entries()? {
        let mut entry = entry?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive has more than {MAX_ENTRIES} entries"
            )));
        }

        let entry_type = entry.header().entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive contains an unsupported entry type {entry_type:?}"
            )));
        }

        let path = entry.path()?.into_owned();
        let relative = safe_archive_path(&path)?;
        let size = entry.header().size()?;
        if size > MAX_ENTRY_BYTES {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive entry {} exceeds the size ceiling",
                relative.display()
            )));
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL_BYTES {
            return Err(BridgeError::Invalid(
                "managed runtime archive exceeds the total size ceiling".into(),
            ));
        }

        let target = destination.join(&relative);
        if entry_type == EntryType::Directory {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Carry only the executable bit, and only for the owner-executable
            // case, rather than trusting an archive's full mode.
            if entry.header().mode()? & 0o111 != 0 {
                let mut permissions = out.metadata()?.permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(&target, permissions)?;
            }
        }
    }
    Ok(())
}

/// Reduce an archive path to a safe relative path, or refuse it.
fn safe_archive_path(path: &Path) -> Result<PathBuf, BridgeError> {
    if path.is_absolute() {
        return Err(BridgeError::Invalid(format!(
            "managed runtime archive contains an absolute path: {}",
            path.display()
        )));
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part.to_str().is_none() {
                    return Err(BridgeError::Invalid(
                        "managed runtime archive contains a non-UTF-8 path".into(),
                    ));
                }
                relative.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(BridgeError::Invalid(format!(
                    "managed runtime archive entry escapes its root: {}",
                    path.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(BridgeError::Invalid(format!(
                    "managed runtime archive entry is not relative: {}",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(BridgeError::Invalid(
            "managed runtime archive contains an empty path".into(),
        ));
    }
    Ok(relative)
}

/// Where a runtime Bridge will launch actually came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeResolution {
    /// A path the user configured. Bridge launches it and never touches it.
    Explicit(PathBuf),
    /// A Bridge-managed, receipt-bound payload.
    Managed(PathBuf),
    /// A copy shipped inside the app bundle.
    Bundled(PathBuf),
    /// Found on PATH. Usable, never Bridge's to remove.
    External(PathBuf),
}

impl RuntimeResolution {
    pub fn path(&self) -> &Path {
        match self {
            Self::Explicit(path)
            | Self::Managed(path)
            | Self::Bundled(path)
            | Self::External(path) => path,
        }
    }

    /// Is this Bridge's to uninstall?
    pub const fn is_bridge_owned(&self) -> bool {
        matches!(self, Self::Managed(_))
    }
}

/// Resolve which copy of a runtime to launch.
///
/// The order is deliberate. A user's explicit configuration outranks everything,
/// including a managed payload, because overriding an explicit choice would be
/// Bridge deciding it knows better. A managed payload outranks a bundled or PATH
/// copy, because installing one is how a user asks for it. A PATH copy is
/// reported as external and is never claimed.
pub fn resolve_runtime(
    agent_id: &str,
    explicit: Option<&Path>,
    store: &ManagedPayloadStore,
    bundled: &[PathBuf],
    system: Option<PathBuf>,
) -> Result<RuntimeResolution, BridgeError> {
    if let Some(explicit) = explicit {
        if inspect_external_runtime(explicit).available {
            return Ok(RuntimeResolution::Explicit(explicit.to_path_buf()));
        }
        return Err(BridgeError::Invalid(format!(
            "{agent_id} is configured to use {} but it is not an executable file",
            explicit.display()
        )));
    }
    if let ManagedPayloadStatus::Installed { entrypoint, .. } = store.status(agent_id)? {
        return Ok(RuntimeResolution::Managed(entrypoint));
    }
    if let Some(found) = bundled
        .iter()
        .find(|candidate| inspect_external_runtime(candidate).available)
    {
        return Ok(RuntimeResolution::Bundled(found.clone()));
    }
    if let Some(system) = system.filter(|path| inspect_external_runtime(path).available) {
        return Ok(RuntimeResolution::External(system));
    }
    Err(BridgeError::Invalid(format!(
        "{agent_id} has no managed payload, bundled copy, or runtime on PATH"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const LOCKFILE: &str = r#"{"lockfileVersion":3,"packages":{"node_modules/x":{"integrity":"sha512-aaa"}}}"#;
    const MANIFEST: &str = r#"{"dependencies":{"x":"1.0.0"}}"#;

    fn release(url: &str, sha256: &str, kind: ArtifactKind) -> RuntimeSource {
        RuntimeSource::ReleaseArtifact {
            url: url.into(),
            sha256: sha256.into(),
            kind,
            entrypoint: PathBuf::from("bin/agent"),
        }
    }

    fn digest_of(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A fetcher that serves fixture bytes. No test reaches the network.
    struct FixtureFetcher {
        bytes: Vec<u8>,
        fail: bool,
    }

    impl ArtifactFetcher for FixtureFetcher {
        fn fetch_to(&self, url: &str, destination: &Path) -> Result<(), BridgeError> {
            if self.fail {
                return Err(BridgeError::Invalid(format!("offline fixture refused {url}")));
            }
            fs::write(destination, &self.bytes)?;
            Ok(())
        }
    }

    fn tarball(entries: &[(&str, &[u8], u32)], specials: &[(tar::EntryType, &str)]) -> Vec<u8> {
        let mut raw = Vec::new();
        for (path, bytes, mode) in entries {
            raw.extend(raw_tar_header(path, b'0', bytes.len() as u64, *mode, ""));
            raw.extend(padded(bytes));
        }
        for (entry_type, path) in specials {
            let typeflag = match *entry_type {
                tar::EntryType::Symlink => b'2',
                tar::EntryType::Link => b'1',
                tar::EntryType::Fifo => b'6',
                tar::EntryType::Char => b'3',
                tar::EntryType::Block => b'4',
                tar::EntryType::Directory => b'5',
                _ => b'0',
            };
            raw.extend(raw_tar_header(path, typeflag, 0, 0o644, "target"));
        }
        raw.extend([0u8; 1024]);
        gzip(&raw)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn padded(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        while out.len() % 512 != 0 {
            out.push(0);
        }
        out
    }

    /// Emit a raw ustar header.
    ///
    /// Hand-rolled on purpose: `tar::Builder` refuses to write `..` or absolute
    /// paths, so it cannot express the archives this module has to defend
    /// against. Building the bytes directly is the only way to prove the
    /// validation runs before anything is written.
    fn raw_tar_header(name: &str, typeflag: u8, size: u64, mode: u32, linkname: &str) -> Vec<u8> {
        fn octal(field: &mut [u8], value: u64) {
            let text = format!("{:0width$o}\0", value, width = field.len() - 1);
            field.copy_from_slice(text.as_bytes());
        }
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut block[100..108], mode as u64);
        octal(&mut block[108..116], 0);
        octal(&mut block[116..124], 0);
        octal(&mut block[124..136], size);
        octal(&mut block[136..148], 0);
        block[156] = typeflag;
        block[157..157 + linkname.len()].copy_from_slice(linkname.as_bytes());
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        for byte in &mut block[148..156] {
            *byte = b' ';
        }
        let checksum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
        block[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        block.to_vec()
    }

    #[test]
    fn runtime_sources_reject_unpinned_and_unsupported_descriptors() {
        let valid_digest = "a".repeat(64);
        assert!(release(
            "https://example.com/a.tar.gz",
            &valid_digest,
            ArtifactKind::TarGz
        )
        .validate()
        .is_ok());

        // Not https.
        assert!(release(
            "http://example.com/a.tar.gz",
            &valid_digest,
            ArtifactKind::TarGz
        )
        .validate()
        .is_err());
        // Digest not a SHA-256.
        assert!(release("https://example.com/a.tar.gz", "abc", ArtifactKind::TarGz)
            .validate()
            .is_err());
        // Archive kind that does not match the URL.
        assert!(release(
            "https://example.com/a.zip",
            &valid_digest,
            ArtifactKind::TarGz
        )
        .validate()
        .is_err());
        // Escaping entrypoint.
        let mut escaping = release(
            "https://example.com/a.tar.gz",
            &valid_digest,
            ArtifactKind::TarGz,
        );
        if let RuntimeSource::ReleaseArtifact { entrypoint, .. } = &mut escaping {
            *entrypoint = PathBuf::from("../outside");
        }
        assert!(escaping.validate().is_err());

        // npm closures must be exactly pinned and must actually pin something.
        let closure = |version: &str, lockfile: &'static str| RuntimeSource::NpmClosure {
            package: "@scope/pkg".into(),
            version: version.into(),
            manifest: MANIFEST,
            lockfile,
            entrypoint: PathBuf::from("node_modules/x/index.mjs"),
        };
        assert!(closure("1.2.3", LOCKFILE).validate().is_ok());
        for range in ["^1.2.3", "~1.2.3", ">=1.2.3", "1.x", "*", "latest", ""] {
            assert!(
                closure(range, LOCKFILE).validate().is_err(),
                "{range} must be refused as unpinned"
            );
        }
        // A lockfile with no integrity hashes pins nothing.
        assert!(closure("1.2.3", r#"{"lockfileVersion":3,"packages":{}}"#)
            .validate()
            .is_err());
    }

    #[test]
    fn release_artifacts_must_match_their_pinned_digest() {
        let fixture = tempfile::tempdir().unwrap();
        let bytes = b"official vendor binary".to_vec();
        let fetcher = FixtureFetcher {
            bytes: bytes.clone(),
            fail: false,
        };

        // Correct digest: staged as a file payload.
        let staged = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::RawBinary,
                entrypoint: PathBuf::from("bin/agent"),
            },
            &fixture.path().join("ok"),
            &fetcher,
        )
        .unwrap();
        assert_eq!(staged.shape, PayloadShape::File);
        assert_eq!(staged.sha256, digest_of(&bytes));
        assert!(staged.source_path.is_file());

        // Wrong digest: refused at the integrity stage, nothing left staged.
        let staging = fixture.path().join("bad");
        let (stage, error) = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent".into(),
                sha256: "b".repeat(64),
                kind: ArtifactKind::RawBinary,
                entrypoint: PathBuf::from("bin/agent"),
            },
            &staging,
            &fetcher,
        )
        .unwrap_err();
        assert_eq!(stage, PrepareStage::Integrity);
        assert!(error.to_string().contains("integrity mismatch"));
        assert!(!staging.join("payload/bin/agent").exists());
        assert!(!staging.join("download.part").exists());
    }

    #[test]
    fn a_failed_fetch_leaves_no_payload_and_names_the_stage() {
        let fixture = tempfile::tempdir().unwrap();
        let (stage, error) = prepare(
            &release("https://example.com/a.tar.gz", &"a".repeat(64), ArtifactKind::TarGz),
            &fixture.path().join("staging"),
            &FixtureFetcher {
                bytes: Vec::new(),
                fail: true,
            },
        )
        .unwrap_err();
        assert_eq!(stage, PrepareStage::Fetch);
        assert_eq!(stage.as_str(), "fetch");
        assert!(error.to_string().contains("offline fixture refused"));
        assert!(!fixture.path().join("staging/unpacked").exists());
    }

    #[test]
    fn archive_extraction_rejects_escaping_and_unsupported_entries() {
        let fixture = tempfile::tempdir().unwrap();
        let sentinel = fixture.path().join("MUST-NOT-EXIST");

        // Each of these must be refused, and none may write outside the target.
        let hostile: Vec<(&str, Vec<u8>)> = vec![
            (
                "parent traversal",
                tarball(&[("../MUST-NOT-EXIST", b"pwned", 0o644)], &[]),
            ),
            (
                "deep traversal",
                tarball(&[("bin/../../MUST-NOT-EXIST", b"pwned", 0o644)], &[]),
            ),
            (
                "absolute path",
                tarball(&[("/tmp/MUST-NOT-EXIST", b"pwned", 0o644)], &[]),
            ),
            (
                "symlink entry",
                tarball(&[], &[(tar::EntryType::Symlink, "link")]),
            ),
            (
                "hardlink entry",
                tarball(&[], &[(tar::EntryType::Link, "hard")]),
            ),
            (
                "fifo entry",
                tarball(&[], &[(tar::EntryType::Fifo, "pipe")]),
            ),
            (
                "char device entry",
                tarball(&[], &[(tar::EntryType::Char, "dev")]),
            ),
        ];

        for (label, bytes) in hostile {
            let archive = fixture.path().join(format!("{}.tgz", label.replace(' ', "-")));
            fs::write(&archive, &bytes).unwrap();
            let destination = fixture.path().join(format!("out-{}", label.replace(' ', "-")));
            let error = extract_tar_gz(&archive, &destination)
                .expect_err(&format!("{label} must be refused"));
            let message = error.to_string();
            assert!(
                message.contains("escapes its root")
                    || message.contains("absolute path")
                    || message.contains("unsupported entry type")
                    || message.contains("not relative"),
                "{label} gave an unhelpful error: {message}"
            );
            assert!(
                !sentinel.exists(),
                "{label} wrote outside the destination — validation ran too late"
            );
            assert!(!Path::new("/tmp/MUST-NOT-EXIST").exists());
        }
    }

    #[test]
    fn extraction_accepts_a_well_formed_archive_and_keeps_the_exec_bit() {
        let fixture = tempfile::tempdir().unwrap();
        let archive = fixture.path().join("good.tgz");
        fs::write(
            &archive,
            tarball(
                &[
                    ("bin/agent", b"#!/bin/sh\nexec agent\n", 0o755),
                    ("README", b"docs", 0o644),
                    ("./lib/support.js", b"module", 0o644),
                ],
                &[],
            ),
        )
        .unwrap();
        let out = fixture.path().join("out");
        extract_tar_gz(&archive, &out).unwrap();

        assert_eq!(fs::read(out.join("README")).unwrap(), b"docs");
        assert!(out.join("lib/support.js").is_file(), "./ prefixes normalize");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert!(mode(&out.join("bin/agent")) & 0o111 != 0, "entrypoint stays executable");
            assert!(
                mode(&out.join("README")) & 0o111 == 0,
                "a non-executable entry must not gain the bit"
            );
        }
    }

    #[test]
    fn extraction_is_bounded_by_entry_size() {
        let fixture = tempfile::tempdir().unwrap();
        // A header claiming more than the per-entry ceiling, without producing
        // the bytes — so the refusal is proven to happen before any writing.
        let mut raw = raw_tar_header("bin/huge", b'0', MAX_ENTRY_BYTES + 1, 0o644, "");
        raw.extend([0u8; 1024]);
        let archive = fixture.path().join("huge.tgz");
        fs::write(&archive, gzip(&raw)).unwrap();

        let error = extract_tar_gz(&archive, &fixture.path().join("out")).unwrap_err();
        assert!(
            error.to_string().contains("size ceiling"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_tar_gz_release_stages_a_directory_payload_with_a_tree_digest() {
        let fixture = tempfile::tempdir().unwrap();
        let bytes = tarball(
            &[("bin/agent", b"#!/bin/sh\n", 0o755), ("VERSION", b"1.0.0", 0o644)],
            &[],
        );
        let staged = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent-1.0.0.tar.gz".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::TarGz,
                entrypoint: PathBuf::from("bin/agent"),
            },
            &fixture.path().join("staging"),
            &FixtureFetcher {
                bytes: bytes.clone(),
                fail: false,
            },
        )
        .unwrap();

        assert_eq!(staged.shape, PayloadShape::Directory);
        assert_eq!(staged.entrypoint, PathBuf::from("bin/agent"));
        assert!(staged.source_path.join("bin/agent").is_file());
        // The digest is the engine's own tree digest, so the payload store will
        // agree with it byte for byte.
        assert_eq!(
            staged.sha256,
            crate::managed_payload::source_digest(
                &staged.source_path,
                PayloadShape::Directory,
                Path::new("bin/agent")
            )
            .unwrap()
        );
        // The downloaded archive is not left lying around in staging.
        assert!(!fixture.path().join("staging/download.part").exists());
    }

    #[test]
    fn an_archive_without_its_entrypoint_is_refused() {
        let fixture = tempfile::tempdir().unwrap();
        let bytes = tarball(&[("README", b"no binary here", 0o644)], &[]);
        let (stage, error) = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/a.tar.gz".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::TarGz,
                entrypoint: PathBuf::from("bin/agent"),
            },
            &fixture.path().join("staging"),
            &FixtureFetcher { bytes, fail: false },
        )
        .unwrap_err();
        assert_eq!(stage, PrepareStage::Extract);
        assert!(error.to_string().contains("entrypoint"), "{error}");
    }
}
