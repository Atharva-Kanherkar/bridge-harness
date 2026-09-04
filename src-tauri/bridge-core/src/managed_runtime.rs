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
    borrow::Cow,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::RwLock,
    time::Duration,
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
/// Ceiling on a single download, enforced while streaming and therefore before
/// the digest can be known.
pub const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
        /// The vendor's own version string for this release.
        ///
        /// Carried rather than derived from the digest: the receipt's version is
        /// what a client displays, and a hash prefix in that field reads as a
        /// broken install next to every other agent's real version.
        version: String,
        /// Path of the executable inside the extracted archive, or the file name
        /// to give a raw binary.
        entrypoint: PathBuf,
    },
    /// An npm dependency closure pinned by an exact version and a lockfile.
    NpmClosure {
        package: String,
        version: String,
        /// Contents of the `package.json` Bridge writes into staging.
        ///
        /// `Cow` rather than `&'static str` so a recipe carried as catalog data
        /// (#164) can produce one of these without leaking. Every built-in
        /// recipe still passes a compiled-in `&'static str`, which borrows.
        manifest: Cow<'static, str>,
        /// Contents of the `package-lock.json` that pins every tarball's
        /// integrity. This is the supply-chain guarantee for this source kind.
        lockfile: Cow<'static, str>,
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
                url,
                sha256,
                kind,
                version,
                ..
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
                if !version_is_path_safe(version) {
                    return Err(BridgeError::Invalid(format!(
                        "managed runtime release version must be a safe path component: \
                         {version:?}"
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
        && !version.bytes().any(|byte| {
            // `x` and `X` are npm wildcards just as much as `*` is: `1.2.x`
            // floats the patch version and would make the installed tree depend
            // on when the user clicked install.
            matches!(
                byte,
                b'^' | b'~' | b'*' | b'>' | b'<' | b'=' | b' ' | b'|' | b'x' | b'X'
            )
        })
        && version.split('.').count() >= 3
        && version
            .split('.')
            .take(3)
            .all(|part| !part.is_empty() && part.bytes().next().is_some_and(|b| b.is_ascii_digit()))
}

/// A version safe to spell as a directory name and to carry in a receipt.
///
/// The payload engine checks the receipt's version, but a staging directory is
/// built from the same string well before any of that runs — so an empty or
/// traversing version would shape a directory and pull a whole artifact into it
/// before the only check fired. The rule is the engine's own, restated at the
/// earlier of the two gates rather than as a second opinion.
fn version_is_path_safe(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 128
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
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
            // `https_only` applies to redirect hops too, so a vendor CDN cannot
            // walk the download down to plaintext. Redirects themselves are
            // allowed but bounded: vendor release URLs commonly redirect to a
            // CDN, and the bytes are verified against a pinned digest before
            // anything uses them, so a redirect cannot substitute content.
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
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
        // Bounded before the digest is known, because an unbounded body would
        // let a hostile or misconfigured endpoint fill the disk before integrity
        // could reject anything.
        let mut file = fs::File::create(destination)?;
        let copied = std::io::copy(
            &mut response.by_ref().take(MAX_DOWNLOAD_BYTES + 1),
            &mut file,
        )
        .map_err(|error| BridgeError::Invalid(format!("managed runtime fetch failed: {error}")))?;
        if copied > MAX_DOWNLOAD_BYTES {
            let _ = fs::remove_file(destination);
            return Err(BridgeError::Invalid(format!(
                "managed runtime download from {url} exceeded {MAX_DOWNLOAD_BYTES} bytes"
            )));
        }
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
    let outcome = prepare_into(source, staging, fetcher);
    if outcome.is_err() {
        // Leave nothing a later attempt at this path could mistake for its own
        // work. Half a download and half an unpacked tree are exactly the inputs
        // that make a retry succeed against the wrong bytes.
        let _ = fs::remove_dir_all(staging);
    }
    outcome
}

fn prepare_into(
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
            ..
        } => {
            let download = staging.join("download.part");
            fetcher
                .fetch_to(url, &download)
                .map_err(|error| (PrepareStage::Fetch, error))?;

            let actual =
                file_digest(&download).map_err(|error| (PrepareStage::Integrity, error))?;
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
                    // Two different digests, and conflating them broke this path:
                    // `actual` is the publisher's digest over the raw bytes, which
                    // is what verifies the download, while the payload engine
                    // recomputes its own shape-aware digest over what was stored.
                    // `StagedRuntime.sha256` has to be the latter or the install
                    // fails its own integrity check.
                    let staged_digest = crate::managed_payload::source_digest(
                        &destination,
                        PayloadShape::File,
                        entrypoint,
                    )
                    .map_err(|error| (PrepareStage::Integrity, error))?;
                    Ok(StagedRuntime {
                        source_path: destination,
                        shape: PayloadShape::File,
                        sha256: staged_digest,
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
            fs::write(tree.join("package.json"), manifest.as_bytes())
                .map_err(|error| (PrepareStage::Extract, error.into()))?;
            fs::write(tree.join("package-lock.json"), lockfile.as_bytes())
                .map_err(|error| (PrepareStage::Extract, error.into()))?;

            // `npm ci` installs exactly the lockfile, verifying each tarball
            // against its recorded integrity. That is the supply-chain check for
            // this source kind; the tree digest below is drift detection.
            let mut command = Command::new("npm");
            command
                .args([
                    "ci",
                    "--omit=dev",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                ])
                .current_dir(&tree);
            // `--ignore-scripts` stops package lifecycle scripts, but it does not
            // stop Node from preloading code named by the *ambient* environment.
            // Anything that can inject into the install has to be cleared here.
            for variable in [
                "NODE_OPTIONS",
                "NODE_REPL_EXTERNAL_MODULE",
                "npm_config_node_options",
                "npm_config_ignore_scripts",
                "npm_config_script_shell",
            ] {
                command.env_remove(variable);
            }
            let output = command.output().map_err(|error| {
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
            prune_npm_bin_shims(&tree).map_err(|error| (PrepareStage::Install, error))?;
            ensure_entrypoint_present(&tree, entrypoint)
                .map_err(|error| (PrepareStage::Install, error))?;
            let digest =
                tree_digest(&tree, entrypoint).map_err(|error| (PrepareStage::Integrity, error))?;
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

/// Ceilings applied while extracting.
///
/// Injectable because the production values are deliberately far larger than any
/// fixture can reach — which is precisely how they went untested. Tests drive the
/// same code path with small values; [`ExtractLimits::default`] is what callers
/// get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtractLimits {
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_entries: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_entry_bytes: MAX_ENTRY_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
            max_entries: MAX_ENTRIES,
        }
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
    extract_tar_gz_with_limits(archive, destination, ExtractLimits::default())
}

pub fn extract_tar_gz_with_limits(
    archive: &Path,
    destination: &Path,
    limits: ExtractLimits,
) -> Result<(), BridgeError> {
    use tar::EntryType;

    fs::create_dir_all(destination)?;
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut total = 0u64;
    let mut count = 0usize;

    for entry in tar.entries()? {
        let mut entry = entry?;
        count += 1;
        if count > limits.max_entries {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive has more than {} entries",
                limits.max_entries
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
        // `Entry::size` is the effective size, which a PAX extended header can
        // set independently of the ustar header field. Charging the header field
        // while copying the effective size let a PAX archive declare 64 bytes and
        // write megabytes.
        let size = entry.size();
        if size > limits.max_entry_bytes {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive entry {} exceeds the size ceiling",
                relative.display()
            )));
        }
        total = total.saturating_add(size);
        if total > limits.max_total_bytes {
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
        // Capped independently of any header claim, so the bytes written can
        // never exceed what was charged against the ceilings above.
        let written = std::io::copy(&mut entry.by_ref().take(size), &mut out)?;
        if written != size {
            return Err(BridgeError::Invalid(format!(
                "managed runtime archive entry {} declared {size} bytes but produced {written}",
                relative.display()
            )));
        }
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

/// The managed root the host registered at boot, if any.
///
/// A process-wide registration rather than a store threaded through every
/// adapter constructor: the adapters are built by [`crate::adapters::AdapterRegistry`]
/// with no access to the data directory, and widening all of those signatures to
/// reach one optional lookup would touch far more than this issue should. The
/// resolution logic itself lives in [`resolve_runtime`], which takes a payload
/// status explicitly and is what the tests drive.
///
/// A lock rather than a `OnceLock` because [`crate::BridgeCore::boot`] can run
/// more than once in a process — every test that boots a core does — and a
/// write-once cell would silently pin the first data directory, leaving a second
/// core resolving payloads out of the first one's storage.
static MANAGED_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Serializes any test that mutates the process-wide registration.
///
/// Lives here rather than inside this module's own test block so `managed_agents`
/// takes the *same* lock. Two modules each with a private mutex would not
/// serialize against each other, which is the race this exists to prevent.
#[cfg(test)]
pub(crate) static MANAGED_ROOT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Register where managed payloads live. Called by `BridgeCore::boot`.
pub fn register_managed_root(root: impl Into<PathBuf>) {
    *MANAGED_ROOT
        .write()
        .unwrap_or_else(|error| error.into_inner()) = Some(root.into());
}

/// Forget any registered root. Used when a core shuts down, and by tests.
pub fn clear_managed_root() {
    *MANAGED_ROOT
        .write()
        .unwrap_or_else(|error| error.into_inner()) = None;
}

pub fn managed_root() -> Option<PathBuf> {
    MANAGED_ROOT
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// The entrypoint of an agent's Bridge-managed payload, if one is installed and
/// healthy.
///
/// Returns `None` — never an error — when there is no managed root, no payload,
/// or the payload needs repair. A managed payload that is not currently usable
/// must fall through to whatever the user already had working, not break the
/// agent.
pub fn managed_entrypoint(agent_id: &str) -> Option<PathBuf> {
    let root = managed_root()?;
    match ManagedPayloadStore::new(root).status(agent_id) {
        Ok(ManagedPayloadStatus::Installed { entrypoint, .. }) => Some(entrypoint),
        _ => None,
    }
}

/// Remove npm's `node_modules/.bin` shim directories.
///
/// npm creates those shims as symlinks, and the payload engine rejects a tree
/// containing any symlink — correctly, since a symlink inside Bridge-owned
/// storage can point anywhere. The shims are pure convenience: Bridge launches a
/// platform binary by path or loads a module directly, and never resolves through
/// `.bin`. They are also regenerable by npm, so removing them loses nothing.
///
/// Only directories named `.bin` directly inside a `node_modules` directory are
/// removed, at any depth, so a package that legitimately ships a `.bin` file
/// elsewhere is left alone.
fn prune_npm_bin_shims(root: &Path) -> Result<(), BridgeError> {
    fn walk(directory: &Path, inside_node_modules: bool) -> Result<(), BridgeError> {
        let children = match fs::read_dir(directory) {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for child in children {
            let child = child?;
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)?;
            let name = child.file_name();
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                if inside_node_modules && name == ".bin" {
                    fs::remove_dir_all(&path)?;
                    continue;
                }
                walk(&path, name == "node_modules")?;
            }
        }
        Ok(())
    }
    walk(root, false)
}

/// How a vendor names the platform component of its per-platform npm package.
///
/// Not one scheme: Anthropic and OpenAI publish `win32-*`, while OpenCode
/// publishes `windows-*` — its own postinstall maps `win32` to `windows`. A
/// single shared suffix silently produced package names that do not exist, so
/// the naming is per-vendor and each recipe asks for its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformNaming {
    /// `darwin-arm64`, `win32-x64` — npm's `process.platform` values.
    NodePlatform,
    /// `darwin-arm64`, `windows-x64` — OpenCode's spelling.
    WindowsSpelled,
}

/// The platform component for `naming` on this host, or `None` where no vendor
/// publishes a build.
pub fn npm_platform_suffix(naming: PlatformNaming) -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "aarch64") => Some(match naming {
            PlatformNaming::NodePlatform => "win32-arm64",
            PlatformNaming::WindowsSpelled => "windows-arm64",
        }),
        ("windows", "x86_64") => Some(match naming {
            PlatformNaming::NodePlatform => "win32-x64",
            PlatformNaming::WindowsSpelled => "windows-x64",
        }),
        _ => None,
    }
}

/// Executable file name for this host — vendors ship `.exe` on Windows.
fn executable_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// Rust target triple as Codex's vendor tree names it.
fn codex_vendor_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// Pinned version of the Claude Agent SDK closure.
pub const CLAUDE_SDK_VERSION: &str = "0.3.209";
/// Pinned version of the Codex runtime closure.
pub const CODEX_VERSION: &str = "0.147.0";
/// Pinned version of the OpenCode runtime closure.
pub const OPENCODE_VERSION: &str = "1.18.25";
/// Pinned version of the Cursor agent CLI release.
pub const CURSOR_VERSION: &str = "2026.08.25-3e8eec8";

const CLAUDE_MANIFEST: &str = include_str!("../../../runtimes/claude/package.json");
const CLAUDE_LOCKFILE: &str = include_str!("../../../runtimes/claude/package-lock.json");
const CODEX_MANIFEST: &str = include_str!("../../../runtimes/codex/package.json");
const CODEX_LOCKFILE: &str = include_str!("../../../runtimes/codex/package-lock.json");
const OPENCODE_MANIFEST: &str = include_str!("../../../runtimes/opencode/package.json");
const OPENCODE_LOCKFILE: &str = include_str!("../../../runtimes/opencode/package-lock.json");
#[cfg(test)]
const CLAUDE_SIDECAR_MANIFEST: &str =
    include_str!("../../../sidecar/claude-agent/package.json");
#[cfg(test)]
const CLAUDE_SIDECAR_LOCKFILE: &str =
    include_str!("../../../sidecar/claude-agent/package-lock.json");

/// The Claude Agent SDK closure.
///
/// The entrypoint is the platform package's `claude` executable rather than the
/// SDK's `sdk.mjs`: the receipt's entrypoint has to be an executable file for
/// #167's readiness check, and that binary is what actually runs. The module the
/// sidecar imports is derived from the payload root by [`claude_sdk_module`].
pub fn claude_recipe() -> Option<RuntimeSource> {
    let suffix = npm_platform_suffix(PlatformNaming::NodePlatform)?;
    Some(RuntimeSource::NpmClosure {
        package: "@anthropic-ai/claude-agent-sdk".into(),
        version: CLAUDE_SDK_VERSION.into(),
        manifest: Cow::Borrowed(CLAUDE_MANIFEST),
        lockfile: Cow::Borrowed(CLAUDE_LOCKFILE),
        entrypoint: PathBuf::from("node_modules/@anthropic-ai")
            .join(format!("claude-agent-sdk-{suffix}"))
            .join(executable_name("claude")),
    })
}

/// The ESM entry the Claude sidecar imports, relative to the payload root.
pub fn claude_sdk_module() -> PathBuf {
    PathBuf::from("node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs")
}

/// The Codex runtime closure.
pub fn codex_recipe() -> Option<RuntimeSource> {
    let suffix = npm_platform_suffix(PlatformNaming::NodePlatform)?;
    let triple = codex_vendor_triple()?;
    Some(RuntimeSource::NpmClosure {
        package: "@openai/codex".into(),
        version: CODEX_VERSION.into(),
        manifest: Cow::Borrowed(CODEX_MANIFEST),
        lockfile: Cow::Borrowed(CODEX_LOCKFILE),
        entrypoint: PathBuf::from("node_modules/@openai")
            .join(format!("codex-{suffix}"))
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(executable_name("codex")),
    })
}

/// The OpenCode runtime closure.
///
/// The binary ships inside the platform package, which is why the closure can be
/// installed with `--ignore-scripts`: OpenCode's postinstall only copies that
/// binary into a convenience location Bridge does not use.
pub fn opencode_recipe() -> Option<RuntimeSource> {
    let suffix = npm_platform_suffix(PlatformNaming::WindowsSpelled)?;
    Some(RuntimeSource::NpmClosure {
        package: "opencode-ai".into(),
        version: OPENCODE_VERSION.into(),
        manifest: Cow::Borrowed(OPENCODE_MANIFEST),
        lockfile: Cow::Borrowed(OPENCODE_LOCKFILE),
        entrypoint: PathBuf::from(format!("node_modules/opencode-{suffix}"))
            .join("bin")
            .join(executable_name("opencode")),
    })
}

/// The Cursor agent CLI release artifact.
///
/// Cursor publishes no npm package: the CLI ships as a per-platform tarball from
/// the vendor's own download host, so this is a release artifact pinned by
/// digest per platform rather than a lockfile-pinned closure. The digests were
/// computed from the published tarballs at pin time; a republish under the same
/// version fails integrity on a first install, and on a reinstall is treated as
/// a superseded pin because the digest is part of what the receipt records.
///
/// `None` on Windows: the vendor publishes darwin and linux builds only, so
/// there is nothing to pin there and installing reports the platform as
/// unsupported instead of fetching a url that does not exist.
pub fn cursor_recipe() -> Option<RuntimeSource> {
    let (platform, sha256) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => (
            "darwin/arm64",
            "81d4de7349e208d4ce441ca9c2d4e7d019ec2fbeb1137a79099fd8c4b8662f5f",
        ),
        ("macos", "x86_64") => (
            "darwin/x64",
            "851f5412f603cff4cb37d4d87d5a940c5e642077c0459238398c866a69d3f495",
        ),
        ("linux", "aarch64") => (
            "linux/arm64",
            "f1c1c2330d89fa4ef5b6cc04fcffba15012ff50eacd07e0f3baec0716f25ac5d",
        ),
        ("linux", "x86_64") => (
            "linux/x64",
            "7a212e5a17ff9316f5acc78808e33c536940d5455645022e6388d99ba48c8425",
        ),
        _ => return None,
    };
    Some(RuntimeSource::ReleaseArtifact {
        url: format!(
            "https://downloads.cursor.com/lab/{CURSOR_VERSION}/{platform}/agent-cli-package.tar.gz"
        ),
        sha256: sha256.into(),
        kind: ArtifactKind::TarGz,
        version: CURSOR_VERSION.to_owned(),
        entrypoint: PathBuf::from("dist-package").join("cursor-agent"),
    })
}

/// Every built-in managed runtime, by agent id.
pub fn builtin_recipes() -> Vec<(&'static str, RuntimeSource)> {
    [
        ("claude", claude_recipe()),
        ("codex", codex_recipe()),
        ("cursor", cursor_recipe()),
        ("opencode", opencode_recipe()),
    ]
    .into_iter()
    .filter_map(|(id, source)| source.map(|source| (id, source)))
    .collect()
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
///
/// Takes the payload status the caller has already computed rather than a store to
/// look it up in. Verifying a managed payload means digesting its tree, and every
/// caller that wants a resolution wants the status too — so owning the lookup here
/// meant the same tree was digested twice to answer one question.
pub fn resolve_runtime(
    agent_id: &str,
    explicit: Option<&Path>,
    payload: &ManagedPayloadStatus,
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
    if let ManagedPayloadStatus::Installed { entrypoint, .. } = payload {
        return Ok(RuntimeResolution::Managed(entrypoint.clone()));
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

    const LOCKFILE: &str =
        r#"{"lockfileVersion":3,"packages":{"node_modules/x":{"integrity":"sha512-aaa"}}}"#;
    const MANIFEST: &str = r#"{"dependencies":{"x":"1.0.0"}}"#;

    fn release(url: &str, sha256: &str, kind: ArtifactKind) -> RuntimeSource {
        RuntimeSource::ReleaseArtifact {
            url: url.into(),
            sha256: sha256.into(),
            kind,
            version: "1.0.0".into(),
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
                return Err(BridgeError::Invalid(format!(
                    "offline fixture refused {url}"
                )));
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
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn padded(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        while !out.len().is_multiple_of(512) {
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
        assert!(
            release("https://example.com/a.tar.gz", "abc", ArtifactKind::TarGz)
                .validate()
                .is_err()
        );
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
            manifest: Cow::Borrowed(MANIFEST),
            lockfile: Cow::Borrowed(lockfile),
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
                version: "1.0.0".into(),
                entrypoint: PathBuf::from("bin/agent"),
            },
            &fixture.path().join("ok"),
            &fetcher,
        )
        .unwrap();
        assert_eq!(staged.shape, PayloadShape::File);
        assert!(staged.source_path.is_file());
        // The staged digest is the engine's shape-aware digest, NOT the raw
        // publisher digest that verified the download. Asserting the raw digest
        // here is what hid a real bug: the engine would have rejected the install.
        assert_ne!(
            staged.sha256,
            digest_of(&bytes),
            "a raw file digest is not what the engine recomputes"
        );
        assert_eq!(
            staged.sha256,
            crate::managed_payload::source_digest(
                &staged.source_path,
                PayloadShape::File,
                Path::new("bin/agent")
            )
            .unwrap()
        );

        // Wrong digest: refused at the integrity stage, nothing left staged.
        let staging = fixture.path().join("bad");
        let (stage, error) = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent".into(),
                sha256: "b".repeat(64),
                kind: ArtifactKind::RawBinary,
                version: "1.0.0".into(),
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
            &release(
                "https://example.com/a.tar.gz",
                &"a".repeat(64),
                ArtifactKind::TarGz,
            ),
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
            let archive = fixture
                .path()
                .join(format!("{}.tgz", label.replace(' ', "-")));
            fs::write(&archive, &bytes).unwrap();
            let destination = fixture
                .path()
                .join(format!("out-{}", label.replace(' ', "-")));
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
        assert!(
            out.join("lib/support.js").is_file(),
            "./ prefixes normalize"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert!(
                mode(&out.join("bin/agent")) & 0o111 != 0,
                "entrypoint stays executable"
            );
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
            &[
                ("bin/agent", b"#!/bin/sh\n", 0o755),
                ("VERSION", b"1.0.0", 0o644),
            ],
            &[],
        );
        let staged = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent-1.0.0.tar.gz".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::TarGz,
                version: "1.0.0".into(),
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

    fn executable_at(path: &Path, bytes: &[u8]) -> PathBuf {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path.to_path_buf()
    }

    /// Install a managed payload for `agent_id` from a fixture binary.
    fn install_managed(store: &ManagedPayloadStore, fixture: &Path, agent_id: &str) -> PathBuf {
        let source = executable_at(&fixture.join(format!("{agent_id}-src")), b"managed runtime");
        let entrypoint = PathBuf::from("bin/agent");
        let recipe = crate::managed_payload::PayloadRecipe {
            agent_id: agent_id.into(),
            version: "1.0.0".into(),
            platform: "darwin-arm64".into(),
            source: format!("npm:{agent_id}@1.0.0"),
            expected_sha256: crate::managed_payload::source_digest(
                &source,
                PayloadShape::File,
                &entrypoint,
            )
            .unwrap(),
            source_path: source,
            shape: PayloadShape::File,
            entrypoint,
        };
        let receipt = store.install(&recipe).unwrap().receipt().clone();
        store.root().join(&receipt.entrypoint)
    }

    /// The observation `resolve_runtime` now takes, read from a store.
    ///
    /// Naming it at each call site keeps these tests honest about the fact that
    /// resolution reads a snapshot rather than the filesystem.
    fn observed(store: &ManagedPayloadStore, agent_id: &str) -> ManagedPayloadStatus {
        store.status(agent_id).unwrap()
    }

    #[test]
    fn resolution_prefers_explicit_then_managed_then_bundled_then_path() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let explicit = executable_at(&fixture.path().join("custom/agent"), b"custom");
        let bundled = executable_at(&fixture.path().join("bundle/agent"), b"bundled");
        let system = executable_at(&fixture.path().join("usr/local/bin/agent"), b"system");
        let managed = install_managed(&store, fixture.path(), "codex");

        // All four tiers present.
        assert_eq!(
            resolve_runtime(
                "codex",
                Some(&explicit),
                &observed(&store, "codex"),
                std::slice::from_ref(&bundled),
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::Explicit(explicit.clone())
        );
        // No explicit config: the managed payload wins.
        assert_eq!(
            resolve_runtime(
                "codex",
                None,
                &observed(&store, "codex"),
                std::slice::from_ref(&bundled),
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::Managed(managed.clone())
        );
        // No managed payload: the bundled copy.
        let empty = ManagedPayloadStore::new(fixture.path().join("empty"));
        assert_eq!(
            resolve_runtime(
                "codex",
                None,
                &observed(&empty, "codex"),
                std::slice::from_ref(&bundled),
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::Bundled(bundled)
        );
        // Nothing but PATH: external, and explicitly not owned.
        let resolution = resolve_runtime(
            "codex",
            None,
            &observed(&empty, "codex"),
            &[],
            Some(system.clone()),
        )
        .unwrap();
        assert_eq!(resolution, RuntimeResolution::External(system));
        assert!(!resolution.is_bridge_owned());
        // Nothing at all: an error naming the agent.
        let error =
            resolve_runtime("codex", None, &observed(&empty, "codex"), &[], None).unwrap_err();
        assert!(error.to_string().contains("codex"), "{error}");

        // A configured path that is not executable is an error, not a silent
        // fallback: the user asked for that binary specifically.
        let broken = fixture.path().join("custom/missing");
        assert!(resolve_runtime(
            "codex",
            Some(&broken),
            &observed(&store, "codex"),
            &[],
            None
        )
        .is_err());
    }

    #[test]
    fn path_runtimes_are_never_claimed_as_managed() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let system = executable_at(&fixture.path().join("usr/bin/agent"), b"user's own copy");

        let resolution = resolve_runtime(
            "codex",
            None,
            &observed(&store, "codex"),
            &[],
            Some(system.clone()),
        )
        .unwrap();
        assert!(matches!(resolution, RuntimeResolution::External(_)));
        assert!(!resolution.is_bridge_owned());
        // No receipt was written anywhere for it.
        assert!(!store.root().join("agents").exists());
        assert_eq!(
            fs::read(&system).unwrap(),
            b"user's own copy",
            "resolution must not touch the binary"
        );
    }

    #[test]
    fn a_custom_executable_outranks_a_managed_payload_and_is_never_touched() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let explicit = executable_at(&fixture.path().join("custom/agent"), b"user's build");
        install_managed(&store, fixture.path(), "opencode");

        assert_eq!(
            resolve_runtime(
                "opencode",
                Some(&explicit),
                &observed(&store, "opencode"),
                &[],
                None
            )
            .unwrap(),
            RuntimeResolution::Explicit(explicit.clone())
        );
        // Removing the managed payload leaves the configured one untouched.
        store.uninstall("opencode").unwrap();
        assert_eq!(fs::read(&explicit).unwrap(), b"user's build");
        assert_eq!(
            resolve_runtime(
                "opencode",
                Some(&explicit),
                &observed(&store, "opencode"),
                &[],
                None
            )
            .unwrap(),
            RuntimeResolution::Explicit(explicit)
        );
    }

    #[test]
    fn uninstall_falls_back_to_the_external_copy_without_altering_it() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let system = executable_at(&fixture.path().join("usr/bin/agent"), b"external copy");
        let managed = install_managed(&store, fixture.path(), "claude");

        assert_eq!(
            resolve_runtime(
                "claude",
                None,
                &observed(&store, "claude"),
                &[],
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::Managed(managed.clone())
        );

        store.uninstall("claude").unwrap();
        assert!(!managed.exists(), "the managed payload is gone");
        assert_eq!(
            resolve_runtime(
                "claude",
                None,
                &observed(&store, "claude"),
                &[],
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::External(system.clone()),
            "resolution must fall back to what the user already had"
        );
        assert_eq!(fs::read(&system).unwrap(), b"external copy");
    }

    #[test]
    fn a_drifted_managed_payload_falls_back_rather_than_breaking_the_agent() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let system = executable_at(&fixture.path().join("usr/bin/agent"), b"external copy");
        let managed = install_managed(&store, fixture.path(), "codex");
        fs::write(&managed, b"tampered").unwrap();

        // The payload is repairable, not usable, so resolution must not hand it
        // out — but it must not break the agent either.
        assert_eq!(
            resolve_runtime(
                "codex",
                None,
                &observed(&store, "codex"),
                &[],
                Some(system.clone())
            )
            .unwrap(),
            RuntimeResolution::External(system)
        );
    }

    /// Build a PAX record with the self-describing length prefix the format
    /// requires: `"<len> key=value\n"` where `<len>` counts its own digits.
    fn pax_record(key: &str, value: &str) -> String {
        let mut len = 0;
        loop {
            let candidate = format!("{len} {key}={value}\n");
            if candidate.len() == len {
                return candidate;
            }
            len = candidate.len();
        }
    }

    /// PAX extended headers can set an entry's size independently of the ustar
    /// header field. Charging the header field while copying the effective size
    /// let an archive declare a tiny entry and write an unbounded one.
    #[test]
    fn pax_size_overrides_cannot_bypass_the_entry_ceiling() {
        let fixture = tempfile::tempdir().unwrap();
        // The override claims one byte past the ceiling while the ustar header
        // understates it as 64. Charging the header would let this through; only
        // charging the effective size refuses it. No body is needed, because the
        // refusal must happen before anything is written.
        let oversized = (MAX_ENTRY_BYTES + 1).to_string();
        let record = pax_record("size", &oversized);
        let mut raw = raw_tar_header("PaxHeaders/big", b'x', record.len() as u64, 0o644, "");
        raw.extend(padded(record.as_bytes()));
        raw.extend(raw_tar_header("big", b'0', 64, 0o644, ""));
        raw.extend([0u8; 1024]);

        let archive = fixture.path().join("pax.tgz");
        fs::write(&archive, gzip(&raw)).unwrap();
        let out = fixture.path().join("out");
        let error = extract_tar_gz(&archive, &out).expect_err(
            "a PAX size override past the ceiling must be refused, not charged at its \
             understated ustar size",
        );
        assert!(
            error.to_string().contains("size ceiling"),
            "unexpected error: {error}"
        );
        assert!(!out.join("big").exists(), "nothing may be written");
    }

    #[test]
    fn extraction_is_bounded_by_total_size_and_entry_count() {
        let fixture = tempfile::tempdir().unwrap();
        let body = vec![7u8; 512];
        let archive = fixture.path().join("three.tgz");
        fs::write(
            &archive,
            tarball(
                &[
                    ("a", &body, 0o644),
                    ("b", &body, 0o644),
                    ("c", &body, 0o644),
                ],
                &[],
            ),
        )
        .unwrap();

        // The same three-entry archive, refused three different ways.
        let cases = [
            (
                ExtractLimits {
                    max_entries: 2,
                    ..ExtractLimits::default()
                },
                "more than 2 entries",
            ),
            (
                ExtractLimits {
                    max_total_bytes: 1024,
                    ..ExtractLimits::default()
                },
                "total size ceiling",
            ),
            (
                ExtractLimits {
                    max_entry_bytes: 256,
                    ..ExtractLimits::default()
                },
                "size ceiling",
            ),
        ];
        for (limits, expected) in cases {
            let out = fixture.path().join(format!("out-{}", limits.max_entries));
            let error = extract_tar_gz_with_limits(&archive, &out, limits)
                .expect_err(&format!("{expected} must refuse this archive"));
            assert!(
                error.to_string().contains(expected),
                "expected {expected}, got: {error}"
            );
        }

        // And under the production defaults the same archive is fine, so the
        // refusals above are the ceilings and not something else.
        extract_tar_gz(&archive, &fixture.path().join("ok")).unwrap();
        assert_eq!(ExtractLimits::default().max_entry_bytes, MAX_ENTRY_BYTES);
        assert_eq!(ExtractLimits::default().max_total_bytes, MAX_TOTAL_BYTES);
        assert_eq!(ExtractLimits::default().max_entries, MAX_ENTRIES);
    }

    #[test]
    fn a_raw_binary_release_installs_through_the_payload_engine() {
        // The regression for the digest confusion: the publisher's digest verifies
        // the download, and the engine recomputes its own. Handing it the former
        // made every raw-binary install fail its own integrity check.
        let fixture = tempfile::tempdir().unwrap();
        let bytes = b"#!/bin/sh\nexec agent\n".to_vec();
        let staged = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::RawBinary,
                version: "1.0.0".into(),
                entrypoint: PathBuf::from("bin/agent"),
            },
            &fixture.path().join("staging"),
            &FixtureFetcher {
                bytes: bytes.clone(),
                fail: false,
            },
        )
        .unwrap();

        let store = ManagedPayloadStore::new(fixture.path().join("managed"));
        let receipt = store
            .install(&crate::managed_payload::PayloadRecipe {
                agent_id: "raw-agent".into(),
                version: "1.0.0".into(),
                platform: "darwin-arm64".into(),
                source: "https://example.com/agent".into(),
                expected_sha256: staged.sha256.clone(),
                source_path: staged.source_path.clone(),
                shape: staged.shape,
                entrypoint: staged.entrypoint.clone(),
            })
            .expect("a raw-binary release must install through the engine")
            .receipt()
            .clone();
        assert_eq!(receipt.integrity_sha256, staged.sha256);
        assert!(matches!(
            store.status("raw-agent").unwrap(),
            ManagedPayloadStatus::Installed { .. }
        ));
    }

    #[test]
    fn npm_wildcards_are_not_exact_versions() {
        for exact in ["1.2.3", "0.3.209", "1.2.3-beta.1", "10.20.30"] {
            assert!(version_is_exact(exact), "{exact} is exact");
        }
        for floating in [
            "1.2.x", "1.X.3", "1.2.X", "x", "^1.2.3", "~1.2.3", ">=1.2.3", "1.2.*", "*", "latest",
            "", "1.2", "next", "1..3", "v1.2.3",
        ] {
            assert!(!version_is_exact(floating), "{floating} must not be exact");
        }
    }

    #[test]
    fn a_failed_prepare_leaves_no_staging_residue() {
        let fixture = tempfile::tempdir().unwrap();
        let staging = fixture.path().join("staging");
        let bytes = b"vendor bytes".to_vec();

        // Integrity failure: nothing of the attempt may survive for a retry at the
        // same path to adopt.
        let (stage, _) = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/agent".into(),
                sha256: "c".repeat(64),
                kind: ArtifactKind::RawBinary,
                version: "1.0.0".into(),
                entrypoint: PathBuf::from("bin/agent"),
            },
            &staging,
            &FixtureFetcher { bytes, fail: false },
        )
        .unwrap_err();
        assert_eq!(stage, PrepareStage::Integrity);
        assert!(
            !staging.exists(),
            "a failed prepare must leave no residue at its staging path"
        );
    }

    #[test]
    fn platform_naming_follows_each_vendors_own_spelling() {
        // The two schemes differ only on Windows, which is exactly why a single
        // shared suffix silently produced package names that do not exist.
        let node = npm_platform_suffix(PlatformNaming::NodePlatform);
        let windows = npm_platform_suffix(PlatformNaming::WindowsSpelled);
        assert!(
            node.is_some() && windows.is_some(),
            "this host must be supported"
        );
        if cfg!(windows) {
            assert!(node.unwrap().starts_with("win32-"));
            assert!(windows.unwrap().starts_with("windows-"));
            assert_ne!(node, windows);
            // And the executable name carries the extension vendors publish.
            assert_eq!(executable_name("codex"), "codex.exe");
        } else {
            assert_eq!(node, windows, "only Windows spelling differs");
            assert_eq!(executable_name("codex"), "codex");
        }

        // Every host arch the lockfiles pin must resolve, Windows arm64 included.
        for lockfile in [CLAUDE_LOCKFILE, CODEX_LOCKFILE, OPENCODE_LOCKFILE] {
            for arch in ["darwin-arm64", "darwin-x64"] {
                assert!(lockfile.contains(arch), "lockfile must pin {arch}");
            }
        }
        assert!(CLAUDE_LOCKFILE.contains("win32-arm64"));
        assert!(OPENCODE_LOCKFILE.contains("windows-arm64"));
    }

    /// The managed launch tier is only live once a host registers a root, and
    /// nothing registered one — so every adapter's managed preference resolved to
    /// nothing and the whole tier was dead. `BridgeCore::boot` registers it now;
    /// this proves the lookup works once it has.
    ///
    /// Takes the registration lock, since the root is process-wide by design and
    /// this test both sets and clears it.
    #[test]
    fn registering_a_root_makes_the_managed_tier_live() {
        let _guard = MANAGED_ROOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        clear_managed_root();
        assert_eq!(
            managed_entrypoint("codex"),
            None,
            "with no registered root the tier must be inert, not guessing"
        );

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("managed-runtimes");
        let store = ManagedPayloadStore::new(&root);
        let expected = install_managed(&store, fixture.path(), "codex");

        // Still inert until a host registers, which is exactly the bug.
        assert_eq!(managed_entrypoint("codex"), None);

        register_managed_root(&root);
        assert_eq!(
            managed_entrypoint("codex"),
            Some(expected.clone()),
            "a registered root must surface the installed payload"
        );

        // A second registration wins, so a core that boots twice cannot resolve
        // payloads out of the first core's directory.
        let other = tempfile::tempdir().unwrap();
        register_managed_root(other.path().join("managed-runtimes"));
        assert_eq!(managed_entrypoint("codex"), None);

        // Drift falls back rather than handing out a payload that needs repair.
        register_managed_root(&root);
        fs::write(&expected, b"tampered").unwrap();
        assert_eq!(managed_entrypoint("codex"), None);

        clear_managed_root();
    }

    #[test]
    fn npm_bin_symlinks_are_pruned_before_digesting() {
        let fixture = tempfile::tempdir().unwrap();
        let tree = fixture.path().join("closure");
        let modules = tree.join("node_modules");
        fs::create_dir_all(modules.join(".bin")).unwrap();
        fs::create_dir_all(modules.join("pkg/node_modules/.bin")).unwrap();
        fs::create_dir_all(modules.join("pkg/lib")).unwrap();
        // A directory legitimately named .bin that is *not* an npm shim dir.
        fs::create_dir_all(modules.join("pkg/lib/.bin")).unwrap();
        fs::write(modules.join("pkg/lib/.bin/keep"), b"not a shim").unwrap();
        fs::write(modules.join("pkg/index.js"), b"module").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("../pkg/cli.js", modules.join(".bin/tool")).unwrap();
            std::os::unix::fs::symlink("../../cli.js", modules.join("pkg/node_modules/.bin/x"))
                .unwrap();
        }

        // Before pruning the engine refuses the tree, because it contains symlinks.
        #[cfg(unix)]
        assert!(
            crate::managed_payload::source_digest(
                &tree,
                PayloadShape::Directory,
                Path::new("node_modules/pkg/index.js")
            )
            .is_err(),
            "a tree with .bin symlinks must be rejected before pruning"
        );

        prune_npm_bin_shims(&tree).unwrap();

        assert!(!modules.join(".bin").exists());
        assert!(!modules.join("pkg/node_modules/.bin").exists());
        assert!(
            modules.join("pkg/lib/.bin/keep").exists(),
            "a .bin directory that is not an npm shim dir must be left alone"
        );
        assert!(modules.join("pkg/index.js").exists());
        // And now the engine accepts it.
        assert!(crate::managed_payload::source_digest(
            &tree,
            PayloadShape::Directory,
            Path::new("node_modules/pkg/index.js")
        )
        .is_ok());
    }

    /// `manifest` and `lockfile` widened from `&'static str` to `Cow` so a
    /// recipe carried as catalog data (#164) can produce a `RuntimeSource`
    /// without leaking. The built-ins must be untouched by that: same bytes,
    /// and still borrowed rather than copied onto the heap at every call.
    #[test]
    fn built_in_recipes_still_borrow_their_compiled_in_closures() {
        for (agent_id, source) in builtin_recipes() {
            let RuntimeSource::NpmClosure {
                manifest, lockfile, ..
            } = &source
            else {
                // A release artifact carries no closure to borrow.
                continue;
            };
            assert!(
                matches!(manifest, Cow::Borrowed(_)),
                "{agent_id} manifest is allocated; a compiled-in closure must borrow"
            );
            assert!(
                matches!(lockfile, Cow::Borrowed(_)),
                "{agent_id} lockfile is allocated; a compiled-in closure must borrow"
            );
        }
    }

    /// The other half: an *owned* closure — what catalog data produces — is
    /// accepted by exactly the validation the built-ins face, with no separate
    /// path and no weaker rules.
    #[test]
    fn an_owned_closure_faces_the_same_validation_as_a_compiled_in_one() {
        let owned = |version: &str| RuntimeSource::NpmClosure {
            package: "example-agent".into(),
            version: version.to_owned(),
            manifest: Cow::Owned(MANIFEST.to_owned()),
            lockfile: Cow::Owned(LOCKFILE.to_owned()),
            entrypoint: PathBuf::from("node_modules/example-agent/bin/agent"),
        };
        owned("1.2.3")
            .validate()
            .expect("a pinned owned closure installs");
        // And the range that is refused for a built-in is refused here too.
        assert!(owned("^1.2.3").validate().is_err());

        let unpinned = RuntimeSource::NpmClosure {
            package: "example-agent".into(),
            version: "1.2.3".into(),
            manifest: Cow::Owned(MANIFEST.to_owned()),
            lockfile: Cow::Owned("{\"lockfileVersion\":3}".to_owned()),
            entrypoint: PathBuf::from("node_modules/example-agent/bin/agent"),
        };
        assert!(
            unpinned.validate().is_err(),
            "a lockfile with no integrity hashes pins nothing, owned or not"
        );
    }

    fn assert_npm_pin(
        label: &str,
        package: &str,
        expected_version: &str,
        manifest_json: &str,
        lockfile_json: &str,
    ) {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json)
            .unwrap_or_else(|error| panic!("{label} manifest is invalid: {error}"));
        assert_eq!(
            manifest["dependencies"][package].as_str(),
            Some(expected_version),
            "{label} manifest must use the compiled-in exact pin"
        );

        let lockfile: serde_json::Value = serde_json::from_str(lockfile_json)
            .unwrap_or_else(|error| panic!("{label} lockfile is invalid: {error}"));
        assert_eq!(
            lockfile["packages"][""]["dependencies"][package].as_str(),
            Some(expected_version),
            "{label} lockfile root must preserve the exact manifest pin"
        );
        let package_path = format!("node_modules/{package}");
        assert_eq!(
            lockfile["packages"][&package_path]["version"].as_str(),
            Some(expected_version),
            "{label} lockfile must resolve the compiled-in version"
        );
    }

    #[test]
    fn managed_runtime_pins_match_update_manifests() {
        for (label, package, version, manifest, lockfile) in [
            (
                "Claude managed runtime",
                "@anthropic-ai/claude-agent-sdk",
                CLAUDE_SDK_VERSION,
                CLAUDE_MANIFEST,
                CLAUDE_LOCKFILE,
            ),
            (
                "Claude sidecar",
                "@anthropic-ai/claude-agent-sdk",
                CLAUDE_SDK_VERSION,
                CLAUDE_SIDECAR_MANIFEST,
                CLAUDE_SIDECAR_LOCKFILE,
            ),
            (
                "Codex managed runtime",
                "@openai/codex",
                CODEX_VERSION,
                CODEX_MANIFEST,
                CODEX_LOCKFILE,
            ),
            (
                "OpenCode managed runtime",
                "opencode-ai",
                OPENCODE_VERSION,
                OPENCODE_MANIFEST,
                OPENCODE_LOCKFILE,
            ),
        ] {
            assert_npm_pin(label, package, version, manifest, lockfile);
        }
    }

    #[test]
    fn each_agent_recipe_pins_an_exact_version_and_entrypoint() {
        let recipes = builtin_recipes();
        // Three npm closures wherever a vendor publishes at all, plus Cursor
        // wherever its vendor ships a tarball. Counted rather than hardcoded:
        // Cursor has no Windows build, and a fixed number here would fail on a
        // platform whose recipe set is correct.
        assert_eq!(
            recipes.len(),
            3 + usize::from(cursor_recipe().is_some()),
            "every agent with a published build must have a recipe on a supported platform"
        );
        for (agent_id, source) in recipes {
            source
                .validate()
                .unwrap_or_else(|error| panic!("{agent_id} recipe is invalid: {error}"));
            assert_eq!(source.shape(), PayloadShape::Directory);

            let RuntimeSource::NpmClosure {
                package,
                version,
                lockfile,
                entrypoint,
                ..
            } = &source
            else {
                // Cursor is the one vendor with no npm distribution: its recipe
                // is a release artifact whose pinning is the url's version
                // component plus the digest validate() already checked.
                let RuntimeSource::ReleaseArtifact { url, version, .. } = &source else {
                    unreachable!("{agent_id} has an unknown source kind");
                };
                assert_eq!(agent_id, "cursor", "only cursor may skip npm: {url}");
                assert!(
                    url.contains(CURSOR_VERSION),
                    "cursor url does not pin {CURSOR_VERSION}: {url}"
                );
                // The receipt's version is what the runtimes card displays, so it
                // must be the vendor's string rather than a digest prefix.
                assert_eq!(version, CURSOR_VERSION);
                continue;
            };
            assert!(
                version_is_exact(version),
                "{agent_id} version {version} is not exact"
            );
            assert!(
                lockfile.contains("\"integrity\""),
                "{agent_id} lockfile pins no integrity"
            );
            // The lockfile must actually pin the version the recipe claims.
            assert!(
                lockfile.contains(&format!("\"version\": \"{version}\"")),
                "{agent_id} lockfile does not contain version {version}"
            );
            assert!(
                lockfile.contains(package),
                "{agent_id} lockfile does not name {package}"
            );
            // The entrypoint lives inside the closure and names a platform package.
            let entrypoint = entrypoint.to_string_lossy();
            assert!(
                entrypoint.starts_with("node_modules/"),
                "{agent_id}: {entrypoint}"
            );
            // Each vendor's own spelling, and the package must be one the
            // lockfile actually pins — a suffix that does not exist upstream
            // would install nothing.
            let naming = if agent_id == "opencode" {
                PlatformNaming::WindowsSpelled
            } else {
                PlatformNaming::NodePlatform
            };
            let suffix = npm_platform_suffix(naming).unwrap();
            assert!(
                entrypoint.contains(suffix),
                "{agent_id} entrypoint must name the host platform package: {entrypoint}"
            );
            let platform_package = entrypoint
                .split('/')
                .find(|part| part.ends_with(suffix))
                .unwrap_or_default();
            assert!(
                lockfile.contains(platform_package),
                "{agent_id} lockfile does not pin {platform_package}"
            );
        }
    }

    #[test]
    fn a_staged_runtime_installs_through_the_payload_engine() {
        // The bridge between this module and #174, asserted by actually installing
        // rather than by validating a recipe in isolation.
        let fixture = tempfile::tempdir().unwrap();
        let platform = npm_platform_suffix(PlatformNaming::NodePlatform).unwrap();
        let bytes = tarball(&[("bin/codex", b"#!/bin/sh\nexec codex\n", 0o755)], &[]);
        let staged = prepare(
            &RuntimeSource::ReleaseArtifact {
                url: "https://example.com/codex.tar.gz".into(),
                sha256: digest_of(&bytes),
                kind: ArtifactKind::TarGz,
                version: "1.0.0".into(),
                entrypoint: PathBuf::from("bin/codex"),
            },
            &fixture.path().join("staging"),
            &FixtureFetcher { bytes, fail: false },
        )
        .unwrap();

        let store =
            crate::managed_payload::ManagedPayloadStore::new(fixture.path().join("managed"));
        let outcome = store
            .install(&crate::managed_payload::PayloadRecipe {
                agent_id: "codex".into(),
                version: CODEX_VERSION.into(),
                platform: platform.into(),
                source: format!("npm:@openai/codex@{CODEX_VERSION}"),
                expected_sha256: staged.sha256.clone(),
                source_path: staged.source_path.clone(),
                shape: staged.shape,
                entrypoint: staged.entrypoint.clone(),
            })
            .expect("a staged runtime must install through the engine");

        // The digest this module computed is the one the engine recorded, so the
        // two agree without a translation step.
        assert_eq!(outcome.receipt().integrity_sha256, staged.sha256);
        assert!(matches!(
            store.status("codex").unwrap(),
            crate::managed_payload::ManagedPayloadStatus::Installed { .. }
        ));
        // And the platform component is a safe path component for the engine.
        assert!(!platform.contains('/'));
    }

    #[test]
    fn claude_sdk_module_points_inside_the_closure() {
        let module = claude_sdk_module();
        assert!(module.starts_with("node_modules"));
        assert!(module.ends_with("sdk.mjs"));
        // Distinct from the receipt entrypoint, which must be an executable.
        let RuntimeSource::NpmClosure { entrypoint, .. } = claude_recipe().unwrap() else {
            panic!("claude installs from npm");
        };
        assert_ne!(entrypoint, module);
        assert!(entrypoint.ends_with("claude"));
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
                version: "1.0.0".into(),
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
