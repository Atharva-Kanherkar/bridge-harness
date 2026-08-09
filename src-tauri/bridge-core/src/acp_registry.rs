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
use std::collections::BTreeMap;

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
        // The on-disk cache stores a `RegistryIndex` as JSON, and `PlatformTarget`
        // is a *map key* there — a shape serde only accepts because the variants
        // serialize as plain strings. Proving it here keeps the cache from being
        // the place this is discovered.
        let parsed = index();
        let encoded = serde_json::to_string(&parsed).expect("index serializes");
        let decoded: RegistryIndex = serde_json::from_str(&encoded).expect("index deserializes");
        assert_eq!(parsed, decoded);
        assert!(
            encoded.contains("\"darwin-aarch64\""),
            "platform keys must round-trip as their registry spelling"
        );
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
