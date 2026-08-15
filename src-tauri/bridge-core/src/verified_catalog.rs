//! The Bridge Verified compatibility catalog: which agents, at which exact
//! versions, Bridge is willing to support.
//!
//! **Not a mirror.** [`crate::acp_registry`] reads an upstream index and is
//! useful for discovering that an agent exists; nothing in it can produce an
//! entry here. Upstream publication is an input to a decision, never a
//! substitute for one, so there is deliberately no conversion in either
//! direction — a test asserts the absence.
//!
//! **Profiles are data, and only data.** No type in this module has a field
//! that could carry a command, an argv, a script, an interpreter, or a path to
//! execute. Everything executable lives in a named integration module compiled
//! into Bridge. A catalog that could deliver behaviour would be a code-delivery
//! channel wearing a catalog's clothes.
//!
//! **Trust is a signature over exact bytes.** A snapshot is authenticated
//! before it is parsed, validated in full before it replaces anything, and
//! cached as the bytes that were verified rather than as the parse they
//! produced.

use crate::{
    acp_registry::PlatformTarget,
    backend_binding::BackendKind,
    managed_runtime::{ArtifactKind, RuntimeSource},
};
use bridge_protocol::messages::{AgentId, BackendId, BackendVersion};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::BTreeSet, path::PathBuf};

/// The snapshot schema this build understands. A document declaring anything
/// else is refused rather than parsed leniently: a catalog is the one place
/// where "read what you can" is the wrong instinct.
pub const SCHEMA_VERSION: u32 = 1;

/// Maximum size of a catalog snapshot document. The bundled bootstrap is a few
/// kilobytes; two MiB allows very large growth while keeping a hostile or
/// broken publisher from making the desktop buffer an unbounded response.
pub const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;

/// Publishing keys this build trusts.
///
/// **Empty, deliberately.** Release signing is not set up yet, and a placeholder
/// key is strictly worse than no key: it is a public key whose private half
/// nobody holds, so it can never admit a real snapshot, and it invites someone
/// to "fill in the real one later" in a diff that reviews as a constant change.
///
/// An empty trust root fails closed — every remote snapshot is refused with
/// `unknown_key`, and the bundled bootstrap is what serves. That is the correct
/// behaviour for a build that cannot yet authenticate anything, and it is the
/// behaviour `an_empty_trust_root_refuses_every_snapshot` pins.
const TRUSTED_KEYS: &[(&str, [u8; 32])] = &[];

/// The keys a catalog will accept a snapshot from.
///
/// A value rather than a constant so #168's pipeline can verify against a
/// staging key without that key ever being compiled into a shipped build, and
/// so the verification path itself is testable. [`TrustRoot::production`] is
/// the only one a shipped Bridge uses.
#[derive(Debug, Clone, Default)]
pub struct TrustRoot {
    keys: Vec<(String, VerifyingKey)>,
}

impl TrustRoot {
    /// The keys compiled into this build.
    pub fn production() -> Self {
        Self::from_keys(
            TRUSTED_KEYS
                .iter()
                .map(|(id, bytes)| ((*id).to_owned(), *bytes)),
        )
        .expect("compiled-in trusted keys are valid")
    }

    /// A trust root over caller-supplied keys. Fails if any is not a valid
    /// Ed25519 public key, so an unusable root cannot be built and then quietly
    /// refuse everything for the wrong reason.
    pub fn from_keys(
        keys: impl IntoIterator<Item = (String, [u8; 32])>,
    ) -> Result<Self, CatalogError> {
        keys.into_iter()
            .map(|(id, bytes)| {
                VerifyingKey::from_bytes(&bytes)
                    .map(|key| (id, key))
                    .map_err(|_| CatalogError::MalformedSignature)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|keys| Self { keys })
    }

    fn key(&self, key_id: &str) -> Option<&VerifyingKey> {
        self.keys
            .iter()
            .find(|(id, _)| id == key_id)
            .map(|(_, key)| key)
    }
}

/// A snapshot document and the detached signature offered with it.
#[derive(Debug, Clone, Copy)]
pub struct SignedSnapshot<'bytes> {
    pub document: &'bytes [u8],
    pub key_id: &'bytes str,
    pub signature: &'bytes [u8],
}

/// Why a snapshot was refused. One code per condition, so a caller can act on
/// them differently and a UI can say which one happened without parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The document is larger than a catalog has any reason to be.
    TooLarge {
        bytes: usize,
    },
    /// No trusted key produced this signature — including a signature that is
    /// well-formed but over different bytes.
    SignatureRejected,
    /// The signature names a key this build does not carry.
    UnknownKey {
        key_id: String,
    },
    /// The signature is not 64 bytes of Ed25519.
    MalformedSignature,
    /// The document did not parse, or declared a schema this build has no
    /// reader for.
    Unreadable {
        reason: String,
    },
    UnsupportedSchema {
        found: u32,
        supported: u32,
    },
    /// The snapshot is not newer than the one already installed. Refusing this
    /// is what stops a replayed older document reinstating a withdrawn entry.
    NotNewer {
        installed: u64,
        offered: u64,
    },
    /// This Bridge build is outside the snapshot's declared range.
    BridgeVersionUnsupported {
        required: String,
        running: String,
    },
    /// One entry failed validation, which refuses the whole document.
    EntryInvalid {
        agent: String,
        reason: String,
    },
}

impl CatalogError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "snapshot_too_large",
            Self::SignatureRejected => "signature_rejected",
            Self::UnknownKey { .. } => "unknown_key",
            Self::MalformedSignature => "malformed_signature",
            Self::Unreadable { .. } => "snapshot_unreadable",
            Self::UnsupportedSchema { .. } => "unsupported_schema",
            Self::NotNewer { .. } => "snapshot_not_newer",
            Self::BridgeVersionUnsupported { .. } => "bridge_version_unsupported",
            Self::EntryInvalid { .. } => "entry_invalid",
        }
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                formatter,
                "catalog snapshot is {bytes} bytes, over the {MAX_SNAPSHOT_BYTES} limit"
            ),
            Self::SignatureRejected => {
                write!(formatter, "catalog snapshot signature did not verify")
            }
            Self::UnknownKey { key_id } => write!(
                formatter,
                "catalog snapshot is signed by {key_id}, which this build does not trust"
            ),
            Self::MalformedSignature => {
                write!(
                    formatter,
                    "catalog snapshot signature is not a valid Ed25519 signature"
                )
            }
            Self::Unreadable { reason } => {
                write!(formatter, "catalog snapshot could not be read: {reason}")
            }
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "catalog snapshot declares schema {found}; this build reads {supported}"
            ),
            Self::NotNewer { installed, offered } => write!(
                formatter,
                "catalog snapshot generation {offered} does not follow the installed {installed}"
            ),
            Self::BridgeVersionUnsupported { required, running } => write!(
                formatter,
                "catalog snapshot requires Bridge {required}; this build is {running}"
            ),
            Self::EntryInvalid { agent, reason } => {
                write!(formatter, "catalog entry {agent} is invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Whether Bridge has run its own suite against this exact entry.
///
/// Modelled here and *written* by #168. An entry that is not [`Verified`] never
/// loads, so promotion is gated by producing evidence rather than by trusting
/// a reader to filter.
///
/// [`Verified`]: VerificationStatus::Verified
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    /// Verification ran and the entry failed. Kept in the document so a
    /// withdrawal is explicit rather than an absence a reader has to notice.
    Failed,
    /// Queued for verification. Never served.
    Pending,
}

/// What Bridge ran, when, and where the evidence lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub status: VerificationStatus,
    /// Which revision of the conformance suite produced this verdict. A verdict
    /// without it cannot be compared across suite changes.
    pub suite_version: u32,
    pub verified_at: String,
    /// An opaque reference to the evidence #168 produces. Deliberately not a
    /// path or a URL: this must not become a fetch instruction.
    pub evidence_ref: String,
}

/// What the vendor requires before their agent will run. Guidance, never a
/// value — there is no field here a secret could be delivered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VendorRequirement {
    /// The vendor's own login command must have been run.
    VendorLogin,
    /// An environment variable the vendor reads must be set by the user.
    VendorApiKeyEnvironment,
    /// Nothing beyond installing it.
    None,
}

/// How a payload is fetched, as data. Mirrors the shapes
/// [`RuntimeSource`] already validates rather than inventing a second
/// vocabulary; [`CatalogRecipe::to_runtime_source`] is the only conversion, and
/// the result faces exactly the engine's own validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CatalogRecipe {
    #[serde(rename_all = "camelCase")]
    NpmClosure {
        package: String,
        version: String,
        manifest: String,
        lockfile: String,
        /// Module entry inside the installed tree. Relative, and validated as
        /// such by the engine — an absolute or climbing path is refused there.
        entrypoint: String,
    },
    #[serde(rename_all = "camelCase")]
    ReleaseArtifact {
        url: String,
        sha256: String,
        /// `raw_binary` or `tar_gz`. Not an interpreter or a command.
        archive: ArchiveKind,
        entrypoint: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    RawBinary,
    TarGz,
}

impl CatalogRecipe {
    /// Convert to the engine's own source type. The conversion is total; the
    /// *validation* is the engine's, so catalog data cannot get a weaker rule
    /// than a compiled-in recipe.
    pub fn to_runtime_source(&self) -> RuntimeSource {
        match self {
            Self::NpmClosure {
                package,
                version,
                manifest,
                lockfile,
                entrypoint,
            } => RuntimeSource::NpmClosure {
                package: package.clone(),
                version: version.clone(),
                manifest: Cow::Owned(manifest.clone()),
                lockfile: Cow::Owned(lockfile.clone()),
                entrypoint: PathBuf::from(entrypoint),
            },
            Self::ReleaseArtifact {
                url,
                sha256,
                archive,
                entrypoint,
            } => RuntimeSource::ReleaseArtifact {
                url: url.clone(),
                sha256: sha256.clone(),
                kind: match archive {
                    ArchiveKind::RawBinary => ArtifactKind::RawBinary,
                    ArchiveKind::TarGz => ArtifactKind::TarGz,
                },
                entrypoint: PathBuf::from(entrypoint),
            },
        }
    }
}

/// Bridge-owned configuration for an integration. Data the named integration
/// module *reads*; never behaviour it executes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationConfig {
    /// How long to wait for the backend to become answerable, in milliseconds.
    #[serde(default)]
    pub handshake_timeout_ms: Option<u32>,
    /// Whether this backend can serve more than one session per process.
    #[serde(default)]
    pub multiplexes_sessions: bool,
    /// Vendor-specific official methods the integration has a typed handler
    /// for. A method not named here is reported, never dispatched.
    #[serde(default)]
    pub extension_methods: Vec<String>,
}

/// One agent, at one exact version, that Bridge supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifiedEntry {
    pub agent: AgentId,
    pub label: String,
    pub vendor: String,
    pub license: String,
    /// Where the agent itself is published. Shown to a user; never fetched.
    pub source_url: String,
    pub version: BackendVersion,
    pub platforms: Vec<PlatformTarget>,
    pub recipe: CatalogRecipe,
    pub backend: BackendId,
    pub backend_kind: CatalogBackendKind,
    #[serde(default)]
    pub integration: IntegrationConfig,
    pub vendor_requirement: VendorRequirement,
    /// What this entry is expected to be able to do. The profile a runtime's
    /// advertised capabilities are checked *against* — never widened by them.
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub blocked_versions: Vec<BackendVersion>,
    /// Inclusive lower bound of Bridge versions this entry supports.
    pub minimum_bridge_version: String,
    pub verification: Verification,
}

/// The wire spelling of [`BackendKind`], kept separate so the catalog's schema
/// does not move whenever the internal enum is reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogBackendKind {
    SdkSidecar,
    StructuredServer,
    Acp,
    StructuredCli,
}

impl From<CatalogBackendKind> for BackendKind {
    fn from(kind: CatalogBackendKind) -> Self {
        match kind {
            CatalogBackendKind::SdkSidecar => Self::SdkSidecar,
            CatalogBackendKind::StructuredServer => Self::StructuredServer,
            CatalogBackendKind::Acp => Self::Acp,
            CatalogBackendKind::StructuredCli => Self::StructuredCli,
        }
    }
}

impl VerifiedEntry {
    /// Every reason this entry may not be served. Checked before any entry in a
    /// snapshot is served, because one bad entry refuses the whole document.
    fn validate(&self, running_bridge_version: &str) -> Result<(), CatalogError> {
        let invalid = |reason: String| CatalogError::EntryInvalid {
            agent: self.agent.as_str().to_owned(),
            reason,
        };

        // Unverified entries never load. #168 promotes by writing this field;
        // nothing downstream is trusted to filter on it.
        if self.verification.status != VerificationStatus::Verified {
            return Err(invalid(format!(
                "verification status is {:?}, not Verified",
                self.verification.status
            )));
        }
        if self.verification.evidence_ref.trim().is_empty() {
            return Err(invalid("verification carries no evidence reference".into()));
        }
        if self.platforms.is_empty() {
            return Err(invalid("supports no platform".into()));
        }
        if self.blocked_versions.contains(&self.version) {
            return Err(invalid(format!(
                "version {} is in its own blocked list",
                self.version
            )));
        }
        if self.capabilities.is_empty() {
            return Err(invalid(
                "declares no capability, so nothing can be checked against it".into(),
            ));
        }
        if version_is_below(running_bridge_version, &self.minimum_bridge_version) {
            return Err(invalid(format!(
                "requires Bridge {} or newer",
                self.minimum_bridge_version
            )));
        }
        // The recipe faces the engine's validation, not a copy of it.
        self.recipe
            .to_runtime_source()
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        Ok(())
    }

    /// Whether this entry can be installed on the machine it is read on.
    pub fn installable_here(&self) -> bool {
        PlatformTarget::current().is_some_and(|current| self.platforms.contains(&current))
    }
}

/// A signed set of entries. The unit of trust: a snapshot is verified,
/// validated, and installed whole or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogSnapshot {
    pub schema_version: u32,
    /// Monotonic. A snapshot that does not advance it is refused, which is what
    /// keeps a replayed older document from reinstating a withdrawn entry.
    pub generation: u64,
    pub published_at: String,
    /// The oldest Bridge this whole document is meant for. Separate from an
    /// entry's own floor: this one says "do not read me at all", which is the
    /// only way a future schema change can be announced to a build that
    /// predates it.
    pub minimum_bridge_version: String,
    pub entries: Vec<VerifiedEntry>,
}

/// How a catalog came to be trusted: what was verified, by which key, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    /// SHA-256 of the exact bytes whose signature verified.
    pub document_sha256: String,
    pub key_id: String,
    pub generation: u64,
    pub installed_at: String,
    /// True for the compiled-in bootstrap, which is trusted by being part of
    /// the binary rather than by a signature over it.
    pub bundled: bool,
}

/// The catalog in force, and the record of why it is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    snapshot: CatalogSnapshot,
    provenance: Provenance,
}

impl Catalog {
    pub fn entries(&self) -> &[VerifiedEntry] {
        &self.snapshot.entries
    }

    pub fn generation(&self) -> u64 {
        self.snapshot.generation
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// The entry serving one agent, if the catalog has one.
    pub fn entry(&self, agent: &AgentId) -> Option<&VerifiedEntry> {
        self.snapshot
            .entries
            .iter()
            .find(|entry| &entry.agent == agent)
    }

    /// The compiled-in last-known-good catalog.
    ///
    /// It passes exactly the validation a remote snapshot must pass — a
    /// bootstrap that could not be installed as a snapshot is not a catalog,
    /// and `the_bundled_bootstrap_catalog_validates` fails the build if it ever
    /// stops being one.
    pub fn bundled(running_bridge_version: &str) -> Result<Self, CatalogError> {
        let snapshot = parse_and_validate(BUNDLED_CATALOG, running_bridge_version)?;
        Ok(Self {
            provenance: Provenance {
                document_sha256: sha256_hex(BUNDLED_CATALOG),
                key_id: "bundled".into(),
                generation: snapshot.generation,
                installed_at: chrono::Utc::now().to_rfc3339(),
                bundled: true,
            },
            snapshot,
        })
    }

    /// Authenticate, validate, and install a snapshot over this catalog.
    ///
    /// Order matters and is the point: the signature is checked over the exact
    /// bytes *before* anything parses them, the document is validated in full
    /// before anything is served, and only then does the catalog change. Any
    /// refusal leaves `self` exactly as it was — the caller keeps serving
    /// last-known-good.
    pub fn install_snapshot(
        &self,
        offered: SignedSnapshot<'_>,
        trust: &TrustRoot,
        running_bridge_version: &str,
    ) -> Result<Self, CatalogError> {
        if offered.document.len() > MAX_SNAPSHOT_BYTES {
            return Err(CatalogError::TooLarge {
                bytes: offered.document.len(),
            });
        }
        verify_signature(offered, trust)?;

        let text =
            std::str::from_utf8(offered.document).map_err(|error| CatalogError::Unreadable {
                reason: error.to_string(),
            })?;
        let snapshot = parse_and_validate(text, running_bridge_version)?;
        if snapshot.generation <= self.snapshot.generation {
            return Err(CatalogError::NotNewer {
                installed: self.snapshot.generation,
                offered: snapshot.generation,
            });
        }
        Ok(Self {
            provenance: Provenance {
                document_sha256: sha256_hex(text),
                key_id: offered.key_id.to_owned(),
                generation: snapshot.generation,
                installed_at: chrono::Utc::now().to_rfc3339(),
                bundled: false,
            },
            snapshot,
        })
    }
}

/// Verify a detached Ed25519 signature over exactly these bytes.
fn verify_signature(offered: SignedSnapshot<'_>, trust: &TrustRoot) -> Result<(), CatalogError> {
    let key = trust
        .key(offered.key_id)
        .ok_or_else(|| CatalogError::UnknownKey {
            key_id: offered.key_id.to_owned(),
        })?;
    let signature: [u8; 64] = offered
        .signature
        .try_into()
        .map_err(|_| CatalogError::MalformedSignature)?;
    key.verify(offered.document, &Signature::from_bytes(&signature))
        .map_err(|_| CatalogError::SignatureRejected)
}

/// Parse a document and validate every entry. All-or-nothing: one invalid entry
/// refuses the whole snapshot, because a partially applied catalog is one
/// nobody can reason about.
fn parse_and_validate(
    text: &str,
    running_bridge_version: &str,
) -> Result<CatalogSnapshot, CatalogError> {
    let snapshot: CatalogSnapshot =
        serde_json::from_str(text).map_err(|error| CatalogError::Unreadable {
            reason: error.to_string(),
        })?;
    if snapshot.schema_version != SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedSchema {
            found: snapshot.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    if version_is_below(running_bridge_version, &snapshot.minimum_bridge_version) {
        return Err(CatalogError::BridgeVersionUnsupported {
            required: snapshot.minimum_bridge_version.clone(),
            running: running_bridge_version.to_owned(),
        });
    }
    let mut seen = BTreeSet::new();
    for entry in &snapshot.entries {
        if !seen.insert(entry.agent.clone()) {
            return Err(CatalogError::EntryInvalid {
                agent: entry.agent.as_str().to_owned(),
                reason: "appears twice in one snapshot".into(),
            });
        }
        entry.validate(running_bridge_version)?;
    }
    Ok(snapshot)
}

fn sha256_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Whether `running` sorts below `required`, comparing dotted numeric parts.
/// Deliberately simple: Bridge's own version is the only thing compared here,
/// and it is a plain `major.minor.patch` this repository controls.
fn version_is_below(running: &str, required: &str) -> bool {
    let parts = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (running, required) = (parts(running), parts(required));
    for index in 0..running.len().max(required.len()) {
        let (left, right) = (
            running.get(index).copied().unwrap_or(0),
            required.get(index).copied().unwrap_or(0),
        );
        if left != right {
            return left < right;
        }
    }
    false
}

/// The compiled-in last-known-good catalog.
///
/// Empty of agents by design. #171 states that adding a specific agent is not
/// part of this epic, and #166's acceptance demonstrates the framework with
/// *fake* integrations — so shipping a real marketplace agent here would be the
/// one thing every issue in this epic says not to do. The bootstrap exists so
/// that a build with no snapshot yet has a valid catalog to serve rather than
/// an absence every caller has to special-case.
pub const BUNDLED_CATALOG: &str = r#"{
  "schemaVersion": 1,
  "generation": 1,
  "publishedAt": "2026-08-15T00:00:00Z",
  "minimumBridgeVersion": "0.1.0",
  "entries": []
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const BRIDGE: &str = "0.1.0";

    /// A deterministic signing key for tests. Deterministic on purpose: a test
    /// key with a known seed is a *test* key, and the production trust root is
    /// empty precisely so this one can never admit anything in a shipped build.
    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn trust() -> TrustRoot {
        TrustRoot::from_keys([("test-key".to_owned(), test_key().verifying_key().to_bytes())])
            .unwrap()
    }

    fn sign(document: &str) -> Vec<u8> {
        test_key().sign(document.as_bytes()).to_bytes().to_vec()
    }

    fn offered<'a>(document: &'a str, signature: &'a [u8]) -> SignedSnapshot<'a> {
        SignedSnapshot {
            document: document.as_bytes(),
            key_id: "test-key",
            signature,
        }
    }

    /// One valid entry for the named agent. `overrides` is appended as extra
    /// JSON fields, so a test can add one without restating the whole entry.
    fn entry(agent: &str, overrides: &str) -> String {
        format!(
            r#"{{
    "agent": "{agent}",
    "label": "Example",
    "vendor": "Example Inc",
    "license": "Apache-2.0",
    "sourceUrl": "https://example.test/agent",
    "version": "1.2.3",
    "platforms": ["darwin-aarch64", "linux-x86_64"],
    "recipe": {{
      "kind": "npmClosure",
      "package": "example-agent",
      "version": "1.2.3",
      "manifest": "{{\"dependencies\":{{\"example-agent\":\"1.2.3\"}}}}",
      "lockfile": "{{\"lockfileVersion\":3,\"packages\":{{\"node_modules/example-agent\":{{\"integrity\":\"sha512-aaa\",\"version\":\"1.2.3\"}}}}}}",
      "entrypoint": "node_modules/example-agent/bin/agent"
    }},
    "backend": "{agent}.acp",
    "backendKind": "acp",
    "vendorRequirement": "vendor_login",
    "capabilities": ["messages", "streaming"],
    "minimumBridgeVersion": "0.1.0",
    "verification": {{
      "status": "verified",
      "suiteVersion": 1,
      "verifiedAt": "2026-08-15T00:00:00Z",
      "evidenceRef": "evidence-0001"
    }}{overrides}
  }}"#
        )
    }

    /// A snapshot document at the given generation over the given entries.
    /// Composed rather than string-surgeried, so a test that wants two entries
    /// says so instead of splicing braces.
    fn document(generation: u64, entries: &[String]) -> String {
        format!(
            r#"{{
  "schemaVersion": 1,
  "generation": {generation},
  "publishedAt": "2026-08-15T00:00:00Z",
  "minimumBridgeVersion": "0.1.0",
  "entries": [{}]
}}"#,
            entries.join(", ")
        )
    }

    /// The common case: one valid entry named `example`.
    fn snapshot_with(generation: u64, entry_overrides: &str) -> String {
        document(generation, &[entry("example", entry_overrides)])
    }

    fn bundled() -> Catalog {
        Catalog::bundled(BRIDGE).expect("the bundled bootstrap must load")
    }

    fn install(document: &str) -> Result<Catalog, CatalogError> {
        let signature = sign(document);
        bundled().install_snapshot(offered(document, &signature), &trust(), BRIDGE)
    }

    #[test]
    fn the_bundled_bootstrap_catalog_validates() {
        // A bootstrap that could not be installed as a snapshot is not a
        // catalog. Prove it by running the real thing through the same parse
        // and validation a remote document faces.
        let catalog = bundled();
        assert_eq!(catalog.generation(), 1);
        assert!(catalog.provenance().bundled);
        assert_eq!(catalog.provenance().key_id, "bundled");
        assert_eq!(
            catalog.provenance().document_sha256,
            sha256_hex(BUNDLED_CATALOG),
            "provenance must digest the bytes that were actually read"
        );
        parse_and_validate(BUNDLED_CATALOG, BRIDGE)
            .expect("bootstrap must pass snapshot validation");

        // Empty of agents on purpose: #171 says this epic adds no agent.
        assert!(
            catalog.entries().is_empty(),
            "the bootstrap must not ship a marketplace agent"
        );
    }

    #[test]
    fn a_snapshot_must_be_signed_by_a_known_key() {
        let document = snapshot_with(2, "");
        let good = sign(&document);
        assert!(
            install(&document).is_ok(),
            "a correctly signed snapshot installs"
        );

        // A signature over different bytes.
        let elsewhere = sign(&snapshot_with(3, ""));
        let error = bundled()
            .install_snapshot(offered(&document, &elsewhere), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "signature_rejected");

        // A key this trust root does not carry.
        let error = bundled()
            .install_snapshot(
                SignedSnapshot {
                    document: document.as_bytes(),
                    key_id: "some-other-key",
                    signature: &good,
                },
                &trust(),
                BRIDGE,
            )
            .unwrap_err();
        assert_eq!(error.code(), "unknown_key");

        // A signature that is not an Ed25519 signature at all.
        let error = bundled()
            .install_snapshot(offered(&document, b"short"), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "malformed_signature");

        // In every refusal, last-known-good is untouched — install_snapshot
        // returns a new catalog rather than mutating, so the caller keeps
        // serving what it had.
        assert!(bundled().entries().is_empty());
    }

    #[test]
    fn an_empty_trust_root_refuses_every_snapshot() {
        // What a shipped build does today: it cannot authenticate anything, so
        // it authenticates nothing and serves the bootstrap.
        let document = snapshot_with(2, "");
        let signature = sign(&document);
        let error = bundled()
            .install_snapshot(
                offered(&document, &signature),
                &TrustRoot::production(),
                BRIDGE,
            )
            .unwrap_err();
        assert_eq!(error.code(), "unknown_key");
    }

    #[test]
    fn signature_is_verified_over_exact_bytes_before_parsing() {
        // Not even valid JSON. It must be refused for its signature, which is
        // the proof that nothing parses a document Bridge has not authenticated.
        let garbage = "}{ this is not json";
        let error = bundled()
            .install_snapshot(offered(garbage, &[0u8; 64]), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "signature_rejected");

        // And once signed, the same garbage fails at the parse instead — the
        // two stages are ordered, not merged.
        let signature = sign(garbage);
        let error = bundled()
            .install_snapshot(offered(garbage, &signature), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "snapshot_unreadable");
    }

    #[test]
    fn one_bad_entry_rejects_the_whole_snapshot() {
        // Start from a catalog that is already serving something, so "the
        // previous catalog keeps serving" is an observable claim rather than a
        // statement about an empty bootstrap.
        let installed = install(&snapshot_with(2, "")).unwrap();

        // Five entries, one of them never verified. Not one of the four good
        // ones loads: a partially applied catalog is one nobody can reason about.
        let mut entries: Vec<String> = (0..4)
            .map(|index| entry(&format!("fresh{index}"), ""))
            .collect();
        entries
            .push(entry("broken", "").replace(r#""status": "verified""#, r#""status": "pending""#));
        let document = document(3, &entries);
        let signature = sign(&document);

        let error = installed
            .install_snapshot(offered(&document, &signature), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "entry_invalid");
        assert!(error.to_string().contains("broken"), "{error}");

        assert_eq!(
            installed.generation(),
            2,
            "the refused snapshot changed nothing"
        );
        assert!(
            installed
                .entry(&AgentId::parse("fresh0").unwrap())
                .is_none(),
            "a valid entry beside the invalid one must not have loaded either"
        );
        assert!(
            installed
                .entry(&AgentId::parse("example").unwrap())
                .is_some(),
            "last-known-good must still be serving"
        );
    }

    #[test]
    fn an_unverified_entry_never_loads() {
        for status in ["pending", "failed"] {
            let document = snapshot_with(2, "").replace(
                r#""status": "verified""#,
                &format!(r#""status": "{status}""#),
            );
            let error = install(&document).unwrap_err();
            assert_eq!(error.code(), "entry_invalid", "{status} must not load");
        }
        // And a verified entry with no evidence behind it is not verified.
        let document = snapshot_with(2, "")
            .replace(r#""evidenceRef": "evidence-0001""#, r#""evidenceRef": """#);
        assert_eq!(install(&document).unwrap_err().code(), "entry_invalid");
    }

    #[test]
    fn a_snapshot_cannot_roll_the_catalog_back() {
        let second = snapshot_with(2, "");
        let installed = install(&second).unwrap();
        assert_eq!(installed.generation(), 2);

        // Replaying the same document, correctly signed, is refused.
        let signature = sign(&second);
        let error = installed
            .install_snapshot(offered(&second, &signature), &trust(), BRIDGE)
            .unwrap_err();
        assert_eq!(error.code(), "snapshot_not_newer");

        // As is an older one — which is how a withdrawn entry stays withdrawn.
        let first = snapshot_with(1, "");
        let signature = sign(&first);
        assert_eq!(
            installed
                .install_snapshot(offered(&first, &signature), &trust(), BRIDGE)
                .unwrap_err()
                .code(),
            "snapshot_not_newer"
        );
        assert_eq!(
            installed.generation(),
            2,
            "the installed catalog is unchanged"
        );
    }

    #[test]
    fn a_snapshot_outside_this_builds_version_range_is_refused() {
        let document = snapshot_with(2, "").replace(
            r#""minimumBridgeVersion": "0.1.0",
  "entries""#,
            r#""minimumBridgeVersion": "9.0.0",
  "entries""#,
        );
        let error = install(&document).unwrap_err();
        assert_eq!(error.code(), "bridge_version_unsupported");
        assert!(
            error.to_string().contains("9.0.0"),
            "{error}: must name the range"
        );
    }

    #[test]
    fn an_entry_requiring_a_newer_bridge_is_refused_as_an_entry() {
        // Separate from the document-level floor: this one says "this agent
        // needs a newer Bridge", not "do not read this document".
        let document = snapshot_with(2, "").replace(
            r#""minimumBridgeVersion": "0.1.0",
    "verification""#,
            r#""minimumBridgeVersion": "9.0.0",
    "verification""#,
        );
        assert_eq!(install(&document).unwrap_err().code(), "entry_invalid");
    }

    #[test]
    fn an_unsupported_schema_is_refused_rather_than_read_leniently() {
        let document =
            snapshot_with(2, "").replace(r#""schemaVersion": 1"#, r#""schemaVersion": 2"#);
        assert_eq!(install(&document).unwrap_err().code(), "unsupported_schema");
    }

    #[test]
    fn a_duplicate_agent_in_one_snapshot_is_refused() {
        let doubled = document(2, &[entry("example", ""), entry("example", "")]);
        let error = install(&doubled).unwrap_err();
        assert_eq!(error.code(), "entry_invalid");
        assert!(error.to_string().contains("twice"), "{error}");
    }

    #[test]
    fn a_catalog_recipe_becomes_the_engines_runtime_source() {
        let document = snapshot_with(2, "");
        let catalog = install(&document).unwrap();
        let entry = catalog.entry(&AgentId::parse("example").unwrap()).unwrap();
        let source = entry.recipe.to_runtime_source();
        source
            .validate()
            .expect("a catalog recipe must satisfy the engine's own validation");
        assert_eq!(entry.version.as_str(), "1.2.3");
        assert_eq!(entry.backend.as_str(), "example.acp");
        assert_eq!(BackendKind::from(entry.backend_kind), BackendKind::Acp);

        // Provenance records what admitted it.
        assert_eq!(catalog.provenance().key_id, "test-key");
        assert_eq!(catalog.provenance().document_sha256, sha256_hex(&document));
        assert!(!catalog.provenance().bundled);
    }

    #[test]
    fn an_unpinned_or_insecure_recipe_is_refused_by_the_engines_own_rules() {
        // A floating npm range, refused for catalog data exactly as it is for a
        // built-in — the validation lives in one place.
        let ranged = snapshot_with(2, "").replace(
            r#""package": "example-agent",
      "version": "1.2.3","#,
            r#""package": "example-agent",
      "version": "^1.2.3","#,
        );
        assert_eq!(install(&ranged).unwrap_err().code(), "entry_invalid");

        // And a release artifact over plain http.
        let insecure = snapshot_with(2, "").replace(
            r#""kind": "npmClosure",
      "package": "example-agent",
      "version": "1.2.3",
      "manifest": "{\"dependencies\":{\"example-agent\":\"1.2.3\"}}",
      "lockfile": "{\"lockfileVersion\":3,\"packages\":{\"node_modules/example-agent\":{\"integrity\":\"sha512-aaa\",\"version\":\"1.2.3\"}}}",
      "entrypoint": "node_modules/example-agent/bin/agent""#,
            r#""kind": "releaseArtifact",
      "url": "http://example.test/agent.tar.gz",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "archive": "tar_gz",
      "entrypoint": "bin/agent""#,
        );
        let error = install(&insecure).unwrap_err();
        assert_eq!(error.code(), "entry_invalid");
        assert!(error.to_string().contains("https"), "{error}");
    }

    #[test]
    fn an_entry_that_blocks_its_own_version_is_refused() {
        let document = snapshot_with(
            2,
            r#",
    "blockedVersions": ["1.2.3"]"#,
        );
        assert_eq!(install(&document).unwrap_err().code(), "entry_invalid");
    }

    #[test]
    fn an_entry_with_no_platform_or_no_capability_is_refused() {
        let no_platform = snapshot_with(2, "").replace(
            r#""platforms": ["darwin-aarch64", "linux-x86_64"],"#,
            r#""platforms": [],"#,
        );
        assert_eq!(install(&no_platform).unwrap_err().code(), "entry_invalid");

        let no_capability = snapshot_with(2, "").replace(
            r#""capabilities": ["messages", "streaming"],"#,
            r#""capabilities": [],"#,
        );
        let error = install(&no_capability).unwrap_err();
        assert_eq!(error.code(), "entry_invalid");
        assert!(
            error
                .to_string()
                .contains("nothing can be checked against it"),
            "{error}"
        );
    }

    #[test]
    fn every_refusal_condition_has_its_own_code() {
        let codes = [
            CatalogError::TooLarge { bytes: 1 },
            CatalogError::SignatureRejected,
            CatalogError::UnknownKey { key_id: "k".into() },
            CatalogError::MalformedSignature,
            CatalogError::Unreadable { reason: "r".into() },
            CatalogError::UnsupportedSchema {
                found: 2,
                supported: 1,
            },
            CatalogError::NotNewer {
                installed: 2,
                offered: 1,
            },
            CatalogError::BridgeVersionUnsupported {
                required: "9".into(),
                running: "0".into(),
            },
            CatalogError::EntryInvalid {
                agent: "a".into(),
                reason: "r".into(),
            },
        ]
        .map(|error| error.code());
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            codes.len(),
            "codes must be distinct: {codes:?}"
        );
    }

    #[test]
    fn an_oversized_document_is_refused_before_anything_reads_it() {
        let huge = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
        let error = bundled()
            .install_snapshot(
                SignedSnapshot {
                    document: &huge,
                    key_id: "test-key",
                    signature: &[0u8; 64],
                },
                &trust(),
                BRIDGE,
            )
            .unwrap_err();
        assert_eq!(error.code(), "snapshot_too_large");
    }

    /// This module's own source, with the test module cut off.
    ///
    /// The structural claims below are about the catalog's API, and the tests
    /// are not part of it — a test that names a forbidden identifier in order
    /// to forbid it would otherwise fail on itself.
    fn module_source() -> &'static str {
        include_str!("verified_catalog.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part")
    }

    #[test]
    fn upstream_registry_entries_cannot_become_verified_entries() {
        // The structural claim: no conversion exists in either direction. If
        // one is ever added, this stops compiling or stops being true.
        let source = module_source();
        for forbidden in [
            "RegistryAgent",
            "acp_registry::RegistryAgent",
            "from_registry",
            "impl From<crate::acp_registry",
        ] {
            assert!(
                !source.contains(forbidden),
                "{forbidden:?} appears in the catalog: upstream publication must not \
                 be able to produce a Bridge Verified entry"
            );
        }
        // PlatformTarget is the one thing borrowed from that module, and it is
        // a platform vocabulary rather than an upstream entry.
        assert!(source.contains("acp_registry::PlatformTarget"));
    }

    #[test]
    fn no_catalog_type_can_carry_an_executable_or_a_credential() {
        // Every field name in the module's public types, enumerated from the
        // source. A catalog that could deliver a command would be a
        // code-delivery channel wearing a catalog's clothes.
        let source = module_source();
        let fields: Vec<&str> = source
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("pub ") && line.contains(':') && line.ends_with(','))
            .filter_map(|line| line.trim_start_matches("pub ").split(':').next())
            .collect();
        assert!(
            fields.len() > 15,
            "expected the catalog's fields, found {fields:?}"
        );
        for field in &fields {
            for forbidden in [
                "command",
                "argv",
                "args",
                "script",
                "shell",
                "exec",
                "interpreter",
                "binary",
                "token",
                "secret",
                "password",
                "credential",
                "api_key",
            ] {
                assert!(
                    !field.contains(forbidden),
                    "field {field:?} could carry {forbidden:?}; executable behaviour and \
                     credentials must never arrive as catalog data"
                );
            }
        }
    }
}
