//! The ACP Registry catalog: the upstream index of installable coding agents.
//!
//! Bridge reads the vendor-neutral index published by the Agent Client Protocol
//! project (Apache-2.0, public CDN, no auth) and models it as a catalog. This
//! module owns the wire shapes and a deliberately tolerant parser. It does not
//! install, launch, or authenticate anything — the registry is a catalog, not a
//! license grant, and every agent in it carries its own terms.
//!
//! **The parser follows the data, not the published schema.** Measured against
//! the captured index: `repository` and `website` are documented as required
//! and are absent on 6 and 7 of 38 entries respectively; `sha256` is present on
//! only 48 of 90 binary builds; and an entry may offer *several* install
//! methods at once rather than exactly one. A parser written from the schema
//! doc alone would drop real agents on the floor.

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// The upstream index. Republished hourly from npm, PyPI, and GitHub releases.
pub const REGISTRY_INDEX_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// One of the six platform targets the registry names for binary builds.
///
/// Coverage is uneven — `windows-aarch64` appears on 8 of the 17
/// binary-distributed agents — so "no build for this platform" is an ordinary
/// state a caller must handle, not an edge case.
// Every variant is renamed explicitly: `x86_64` does not survive any of serde's
// automatic casings intact, so a container-level `rename_all` here would be both
// dead and misleading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlatformTarget {
    #[serde(rename = "darwin-aarch64")]
    DarwinAarch64,
    #[serde(rename = "darwin-x86_64")]
    DarwinX86_64,
    #[serde(rename = "linux-aarch64")]
    LinuxAarch64,
    #[serde(rename = "linux-x86_64")]
    LinuxX86_64,
    #[serde(rename = "windows-aarch64")]
    WindowsAarch64,
    #[serde(rename = "windows-x86_64")]
    WindowsX86_64,
}

impl PlatformTarget {
    /// The registry's key for this target.
    pub fn as_key(self) -> &'static str {
        match self {
            Self::DarwinAarch64 => "darwin-aarch64",
            Self::DarwinX86_64 => "darwin-x86_64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::LinuxX86_64 => "linux-x86_64",
            Self::WindowsAarch64 => "windows-aarch64",
            Self::WindowsX86_64 => "windows-x86_64",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "darwin-aarch64" => Some(Self::DarwinAarch64),
            "darwin-x86_64" => Some(Self::DarwinX86_64),
            "linux-aarch64" => Some(Self::LinuxAarch64),
            "linux-x86_64" => Some(Self::LinuxX86_64),
            "windows-aarch64" => Some(Self::WindowsAarch64),
            "windows-x86_64" => Some(Self::WindowsX86_64),
            _ => None,
        }
    }

    /// The target the running host needs, or `None` on a platform the registry
    /// does not name. Never guesses a neighbouring architecture.
    pub fn current() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some(Self::DarwinAarch64),
            ("macos", "x86_64") => Some(Self::DarwinX86_64),
            ("linux", "aarch64") => Some(Self::LinuxAarch64),
            ("linux", "x86_64") => Some(Self::LinuxX86_64),
            ("windows", "aarch64") => Some(Self::WindowsAarch64),
            ("windows", "x86_64") => Some(Self::WindowsX86_64),
            _ => None,
        }
    }
}

/// A downloadable build for one platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryBuild {
    /// Archive URL: `.zip`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz2`, or a raw binary.
    pub archive: String,
    /// Integrity hash when the publisher supplies one. Absent on roughly half of
    /// the builds upstream, so an installer must treat verification as
    /// best-effort-when-present rather than guaranteed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Executable path *inside* the extracted archive.
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// A distribution launched through a package runner (`npx` or `uvx`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDistribution {
    /// Already version-pinned upstream, e.g. `minion-code@0.1.44`.
    pub package: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// Every install method an entry offers.
///
/// Modelled as a set rather than a choice because the registry permits several:
/// `kilo` and `sigit` each ship both a binary and an npm package. Picking one is
/// the installer's job, not the catalog's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npx: Option<PackageDistribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uvx: Option<PackageDistribution>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub binary: BTreeMap<PlatformTarget, BinaryBuild>,
}

impl Distribution {
    /// True when nothing here is installable by any method Bridge understands.
    pub fn is_empty(&self) -> bool {
        self.npx.is_none() && self.uvx.is_none() && self.binary.is_empty()
    }

    /// The binary build for the running host, if this entry ships one.
    pub fn binary_for_current_platform(&self) -> Option<&BinaryBuild> {
        self.binary.get(&PlatformTarget::current()?)
    }

    /// Whether this entry can be installed on the running host at all — a
    /// package runner works anywhere, a binary only where a build exists.
    pub fn installable_on_current_platform(&self) -> bool {
        self.npx.is_some() || self.uvx.is_some() || self.binary_for_current_platform().is_some()
    }
}

/// One agent in the catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryAgent {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Present on 32 of 38 entries despite being documented as required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Present on 31 of 38 entries despite being documented as required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// The agent's own licence, surfaced so a user sees it before installing.
    /// Frequently `proprietary` — the registry is a catalog, not a grant.
    pub license: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub distribution: Distribution,
}

/// An entry the parser refused, kept so the count is visible rather than silent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    /// Position in the upstream `agents` array, for reporting an entry that has
    /// no usable id.
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub reason: String,
}

/// A parsed catalog: what was understood, and what was not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryIndex {
    /// The index format version, e.g. `1.0.0`.
    pub version: String,
    pub agents: Vec<RegistryAgent>,
    /// Entries skipped by this parse. Never silently dropped: a client that
    /// shows a catalog needs to be able to say it is incomplete.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedEntry>,
}

/// The envelope. `extensions` is present upstream and deliberately ignored —
/// it is a separate ACP concept from agents.
#[derive(Deserialize)]
struct RawIndex {
    #[serde(default)]
    version: Option<String>,
    agents: Vec<Value>,
}

/// The per-entry shape, kept separate from [`RegistryAgent`] so a distribution
/// with unrecognized keys can be repaired rather than rejected.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgent {
    id: String,
    name: String,
    version: String,
    description: String,
    repository: Option<String>,
    website: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    license: String,
    icon: Option<String>,
    distribution: BTreeMap<String, Value>,
}

/// Parse the upstream index.
///
/// Fails only when the *document* is unusable. A single bad entry is skipped and
/// counted — the same rule #144 applies to durable replay, for the same reason:
/// one malformed row must not deny a client the other thirty-seven.
pub fn parse_index(raw: &str) -> Result<RegistryIndex, BridgeError> {
    let parsed: RawIndex = serde_json::from_str(raw)
        .map_err(|error| BridgeError::Invalid(format!("ACP registry index is not valid: {error}")))?;

    let mut agents = Vec::with_capacity(parsed.agents.len());
    let mut skipped = Vec::new();

    for (index, entry) in parsed.agents.into_iter().enumerate() {
        // Recovered before deserializing so a rejected entry can still name itself.
        let declared_id = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let raw_agent: RawAgent = match serde_json::from_value(entry) {
            Ok(agent) => agent,
            Err(error) => {
                skipped.push(SkippedEntry {
                    index,
                    id: declared_id,
                    reason: error.to_string(),
                });
                continue;
            }
        };

        let distribution = parse_distribution(&raw_agent.distribution);
        if distribution.is_empty() {
            skipped.push(SkippedEntry {
                index,
                id: Some(raw_agent.id),
                reason: format!(
                    "no install method Bridge understands (offered: {})",
                    describe_keys(&raw_agent.distribution)
                ),
            });
            continue;
        }

        agents.push(RegistryAgent {
            id: raw_agent.id,
            name: raw_agent.name,
            version: raw_agent.version,
            description: raw_agent.description,
            repository: raw_agent.repository,
            website: raw_agent.website,
            authors: raw_agent.authors,
            license: raw_agent.license,
            icon: raw_agent.icon,
            distribution,
        });
    }

    Ok(RegistryIndex {
        version: parsed.version.unwrap_or_else(|| "unknown".into()),
        agents,
        skipped,
    })
}

/// Keep every method that parses; ignore the rest.
///
/// An unrecognized distribution key, or a platform target this build does not
/// know, costs only that one method — never the whole entry. An entry left with
/// nothing installable is skipped by the caller.
fn parse_distribution(raw: &BTreeMap<String, Value>) -> Distribution {
    let package = |key: &str| {
        raw.get(key)
            .and_then(|value| serde_json::from_value::<PackageDistribution>(value.clone()).ok())
    };

    let binary = raw
        .get("binary")
        .and_then(Value::as_object)
        .map(|builds| {
            builds
                .iter()
                .filter_map(|(key, value)| {
                    let target = PlatformTarget::from_key(key)?;
                    let build = serde_json::from_value::<BinaryBuild>(value.clone()).ok()?;
                    Some((target, build))
                })
                .collect()
        })
        .unwrap_or_default();

    Distribution {
        npx: package("npx"),
        uvx: package("uvx"),
        binary,
    }
}

fn describe_keys(raw: &BTreeMap<String, Value>) -> String {
    if raw.is_empty() {
        return "none".into();
    }
    raw.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
}

/// A cached copy of the upstream index.
///
/// The **verbatim document** is stored, not our parse of it. Two reasons: the
/// cache stays a faithful copy of what upstream actually served, and a later
/// improvement to [`parse_index`] takes effect on the next read instead of
/// waiting for a network fetch to re-understand entries we previously skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedCatalog {
    /// RFC 3339, when this document was received.
    pub fetched_at: String,
    /// Where it came from — recorded so a cache written against a different
    /// index URL is not silently reused.
    pub source_url: String,
    /// Upstream's ETag, replayed as `If-None-Match` on the next fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The upstream document, byte for byte.
    pub document: String,
}

/// A catalog handed to a caller, carrying its own freshness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub index: RegistryIndex,
    pub fetched_at: String,
    /// True when this came from cache after a failed refresh. A client must be
    /// able to say the catalog is old rather than quietly showing stale data.
    pub stale: bool,
}

impl CachedCatalog {
    /// Parse the cached document into a catalog.
    pub fn to_catalog(&self, stale: bool) -> Result<Catalog, BridgeError> {
        Ok(Catalog {
            index: parse_index(&self.document)?,
            fetched_at: self.fetched_at.clone(),
            stale,
        })
    }
}

/// Where the cache lives under a data directory.
pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("acp-registry").join("index.json")
}

/// Read the cache, or `None` when there isn't a usable one.
///
/// A missing, unreadable, or corrupt cache is not an error — it is simply the
/// absence of a cache. A half-written or hand-edited file must not be able to
/// keep Bridge from starting, so anything that fails to decode is discarded.
pub fn read_cache(path: &Path) -> Option<CachedCatalog> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the cache atomically.
///
/// Writes a sibling temp file and renames it over the destination, so a crash
/// mid-write leaves either the previous cache or the new one — never a truncated
/// document that would read as corrupt on the next start.
pub fn write_cache(path: &Path, cached: &CachedCatalog) -> Result<(), BridgeError> {
    let parent = path.parent().ok_or_else(|| {
        BridgeError::Invalid(format!("registry cache path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;

    let encoded = serde_json::to_string(cached)
        .map_err(|error| BridgeError::Invalid(format!("cannot encode registry cache: {error}")))?;

    // Same directory as the destination, so the rename stays within one
    // filesystem and is therefore atomic.
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, encoded)?;
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Never leave the scratch file behind to be mistaken for a cache.
            let _ = fs::remove_file(&temp);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../testing/fixtures/acp-registry-v1.json");

    fn index() -> RegistryIndex {
        parse_index(FIXTURE).expect("fixture parses")
    }

    fn agent(id: &str) -> RegistryAgent {
        index()
            .agents
            .into_iter()
            .find(|agent| agent.id == id)
            .unwrap_or_else(|| panic!("fixture has no agent {id}"))
    }

    #[test]
    fn parses_the_live_index_fixture() {
        let index = index();
        assert_eq!(index.version, "1.0.0");
        assert_eq!(index.agents.len(), 38);
        assert!(
            index.skipped.is_empty(),
            "every upstream entry should parse, skipped: {:?}",
            index.skipped
        );
    }

    #[test]
    fn parses_npx_uvx_and_binary_distributions() {
        let npx = agent("claude-acp").distribution.npx.expect("npx");
        assert!(npx.package.starts_with("@agentclientprotocol/claude-agent-acp@"));

        let uvx = agent("minion-code").distribution.uvx.expect("uvx");
        assert!(uvx.package.starts_with("minion-code@"));
        assert_eq!(uvx.args, vec!["acp".to_string()], "uvx args must survive");

        let binary = agent("devin").distribution.binary;
        let build = binary
            .get(&PlatformTarget::DarwinAarch64)
            .expect("devin ships a darwin-aarch64 build");
        assert!(build.archive.ends_with(".tar.gz"));
        assert_eq!(build.cmd, "./bin/devin");
        assert_eq!(build.args, vec!["acp".to_string()]);
    }

    #[test]
    fn agent_offering_both_npx_and_binary_keeps_both() {
        // Modelling `distribution` as a single choice would silently drop one of
        // these, and the installer would lose a working fallback.
        for id in ["kilo", "sigit"] {
            let distribution = agent(id).distribution;
            assert!(distribution.npx.is_some(), "{id} should keep its npx method");
            assert!(
                !distribution.binary.is_empty(),
                "{id} should keep its binary builds"
            );
        }
    }

    #[test]
    fn binary_entry_without_sha256_is_valid() {
        let build = agent("devin")
            .distribution
            .binary
            .remove(&PlatformTarget::DarwinAarch64)
            .expect("build");
        assert!(
            build.sha256.is_none(),
            "devin publishes no hash; requiring one would reject a real agent"
        );
    }

    #[test]
    fn entry_missing_optional_repository_and_website_is_valid() {
        let index = index();
        assert!(
            index.agents.iter().any(|agent| agent.website.is_none()),
            "the fixture should exercise a missing website"
        );
        assert!(
            index.agents.iter().any(|agent| agent.repository.is_none()),
            "the fixture should exercise a missing repository"
        );
    }

    #[test]
    fn npx_entry_with_env_is_valid() {
        let npx = agent("auggie").distribution.npx.expect("npx");
        assert_eq!(
            npx.env.get("AUGMENT_DISABLE_AUTO_UPDATE").map(String::as_str),
            Some("1"),
            "env belongs on package distributions, not only on binary builds"
        );
    }

    #[test]
    fn skips_malformed_entry_and_keeps_the_rest() {
        let raw = r#"{
            "version": "1.0.0",
            "agents": [
                {"id":"good-one","name":"Good","version":"1","description":"d",
                 "license":"MIT","distribution":{"npx":{"package":"good@1"}}},
                {"id":"broken","name":"Broken","description":"missing version",
                 "license":"MIT","distribution":{"npx":{"package":"broken@1"}}},
                {"id":"good-two","name":"Good","version":"1","description":"d",
                 "license":"MIT","distribution":{"npx":{"package":"good2@1"}}}
            ]
        }"#;
        let index = parse_index(raw).expect("document is well formed");
        assert_eq!(
            index.agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["good-one", "good-two"]
        );
        assert_eq!(index.skipped.len(), 1);
        assert_eq!(index.skipped[0].id.as_deref(), Some("broken"));
        assert_eq!(index.skipped[0].index, 1);
        assert!(index.skipped[0].reason.contains("version"));
    }

    #[test]
    fn skips_unknown_platform_target_without_failing_the_entry() {
        let raw = r#"{
            "version": "1.0.0",
            "agents": [{
                "id":"a","name":"A","version":"1","description":"d","license":"MIT",
                "distribution":{"binary":{
                    "darwin-aarch64":{"archive":"https://e/a.zip","cmd":"./a"},
                    "plan9-riscv":{"archive":"https://e/b.zip","cmd":"./b"}
                }}
            }]
        }"#;
        let index = parse_index(raw).expect("parses");
        assert!(index.skipped.is_empty(), "one odd platform must not skip the agent");
        let binary = &index.agents[0].distribution.binary;
        assert_eq!(binary.len(), 1);
        assert!(binary.contains_key(&PlatformTarget::DarwinAarch64));
    }

    #[test]
    fn skips_unknown_distribution_type_as_one_entry() {
        let raw = r#"{
            "version": "1.0.0",
            "agents": [{
                "id":"future","name":"Future","version":"1","description":"d",
                "license":"MIT","distribution":{"flatpak":{"ref":"org.example.Future"}}
            }]
        }"#;
        let index = parse_index(raw).expect("parses");
        assert!(index.agents.is_empty());
        assert_eq!(index.skipped.len(), 1);
        assert_eq!(index.skipped[0].id.as_deref(), Some("future"));
        assert!(
            index.skipped[0].reason.contains("flatpak"),
            "the skip reason should name what was offered: {}",
            index.skipped[0].reason
        );
    }

    #[test]
    fn unknown_distribution_key_alongside_a_known_one_keeps_the_known() {
        let raw = r#"{
            "version": "1.0.0",
            "agents": [{
                "id":"mixed","name":"Mixed","version":"1","description":"d","license":"MIT",
                "distribution":{"flatpak":{"ref":"x"},"npx":{"package":"mixed@1"}}
            }]
        }"#;
        let index = parse_index(raw).expect("parses");
        assert!(index.skipped.is_empty());
        assert_eq!(
            index.agents[0].distribution.npx.as_ref().map(|p| p.package.as_str()),
            Some("mixed@1")
        );
    }

    #[test]
    fn rejects_index_with_no_agents_key() {
        let error = parse_index(r#"{"version":"1.0.0"}"#)
            .expect_err("a document without agents is unusable, not an empty catalog");
        assert!(matches!(error, BridgeError::Invalid(_)));
    }

    #[test]
    fn platform_target_for_current_host_resolves() {
        let current = PlatformTarget::current().expect("test hosts are named by the registry");
        assert_eq!(PlatformTarget::from_key(current.as_key()), Some(current));
    }

    #[test]
    fn binary_build_for_missing_platform_is_none() {
        let distribution = Distribution {
            npx: None,
            uvx: None,
            binary: BTreeMap::new(),
        };
        assert!(distribution.binary_for_current_platform().is_none());
        assert!(!distribution.installable_on_current_platform());
        assert!(distribution.is_empty());
    }

    #[test]
    fn the_parsed_index_survives_a_json_round_trip() {
        // `PlatformTarget` is a *map key* inside `Distribution`, a shape serde
        // only accepts because the variants serialize as plain strings. The RPC
        // layer (#157) has to put this whole type on the wire, so prove it here
        // rather than there.
        let parsed = index();
        let encoded = serde_json::to_string(&parsed).expect("index serializes");
        let decoded: RegistryIndex = serde_json::from_str(&encoded).expect("index deserializes");
        assert_eq!(parsed, decoded);
        assert!(
            encoded.contains("\"darwin-aarch64\""),
            "platform keys must round-trip as their registry spelling"
        );
    }

    fn sample_cache() -> CachedCatalog {
        CachedCatalog {
            fetched_at: "2026-08-09T00:00:00Z".into(),
            source_url: REGISTRY_INDEX_URL.into(),
            etag: Some("\"abc123\"".into()),
            document: FIXTURE.into(),
        }
    }

    #[test]
    fn cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path());
        let cached = sample_cache();

        write_cache(&path, &cached).expect("write");
        let read = read_cache(&path).expect("cache is present");

        assert_eq!(read, cached);
        assert_eq!(read.etag.as_deref(), Some("\"abc123\""));
        assert_eq!(read.to_catalog(false).unwrap().index.agents.len(), 38);
    }

    #[test]
    fn absent_cache_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_cache(&cache_path(dir.path())).is_none());
    }

    #[test]
    fn corrupt_cache_is_discarded_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        for garbage in ["", "{", "not json at all", r#"{"fetchedAt":"x"}"#] {
            fs::write(&path, garbage).unwrap();
            assert!(
                read_cache(&path).is_none(),
                "a cache Bridge cannot decode must read as absent, not panic: {garbage:?}"
            );
        }
    }

    #[test]
    fn a_cached_document_that_no_longer_parses_surfaces_as_an_error() {
        // Distinct from a corrupt *envelope*: the cache decoded fine, but what
        // upstream served is no longer usable. That is worth reporting rather
        // than silently pretending there is no cache.
        let cached = CachedCatalog {
            document: r#"{"version":"1.0.0"}"#.into(),
            ..sample_cache()
        };
        assert!(matches!(
            cached.to_catalog(true),
            Err(BridgeError::Invalid(_))
        ));
    }

    #[test]
    fn successful_write_leaves_no_temp_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path());
        write_cache(&path, &sample_cache()).expect("write");

        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file survived: {leftovers:?}");
    }

    #[test]
    fn a_failed_write_leaves_the_previous_cache_intact() {
        // The observable half of atomicity: a write that cannot complete must
        // not degrade what was already there.
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path());
        write_cache(&path, &sample_cache()).expect("first write");

        // A directory where the destination file should be makes the rename fail
        // without touching the good cache we just wrote.
        let blocked = dir.path().join("acp-registry-blocked").join("index.json");
        fs::create_dir_all(&blocked).unwrap();
        assert!(write_cache(&blocked, &sample_cache()).is_err());

        let survivor = read_cache(&path).expect("the good cache is untouched");
        assert_eq!(survivor, sample_cache());
    }

    #[test]
    fn stale_is_carried_on_the_catalog_not_inferred_by_the_caller() {
        let cached = sample_cache();
        assert!(!cached.to_catalog(false).unwrap().stale);
        let stale = cached.to_catalog(true).unwrap();
        assert!(stale.stale);
        assert_eq!(stale.fetched_at, "2026-08-09T00:00:00Z");
        assert_eq!(stale.index.agents.len(), 38, "stale still means usable");
    }

    #[test]
    fn a_package_runner_is_installable_anywhere_a_binary_is_not() {
        let package = PackageDistribution {
            package: "x@1".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        };
        let npx_only = Distribution {
            npx: Some(package),
            uvx: None,
            binary: BTreeMap::new(),
        };
        assert!(npx_only.installable_on_current_platform());

        // A binary map that omits this host is the common "no build for your
        // platform" state, and must not read as installable.
        let other = match PlatformTarget::current() {
            Some(PlatformTarget::DarwinAarch64) => PlatformTarget::WindowsX86_64,
            _ => PlatformTarget::DarwinAarch64,
        };
        let elsewhere = Distribution {
            npx: None,
            uvx: None,
            binary: BTreeMap::from([(
                other,
                BinaryBuild {
                    archive: "https://e/a.zip".into(),
                    sha256: None,
                    cmd: "./a".into(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                },
            )]),
        };
        assert!(!elsewhere.installable_on_current_platform());
        assert!(!elsewhere.is_empty(), "it exists, it just is not for us");
    }
}
