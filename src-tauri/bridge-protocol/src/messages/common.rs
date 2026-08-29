//! Wire types more than one domain needs.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::MAX_SAFE_INTEGER;

/// A signed integer that can cross a JavaScript boundary without losing bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct JsSafeI64(i64);

impl JsSafeI64 {
    pub fn new(value: i64) -> Result<Self, &'static str> {
        if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err("integer exceeds JavaScript's safe integer range")
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for JsSafeI64 {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for JsSafeI64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(i64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for JsSafeI64 {
    fn schema_name() -> String {
        "JsSafeI64".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "integer",
            "minimum": -MAX_SAFE_INTEGER,
            "maximum": MAX_SAFE_INTEGER,
        }))
        .unwrap()
    }
}

/// A non-negative integer that can cross a JavaScript boundary without losing bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct JsSafeU64(u64);

impl JsSafeU64 {
    pub fn new(value: u64) -> Result<Self, &'static str> {
        if value <= MAX_SAFE_INTEGER as u64 {
            Ok(Self(value))
        } else {
            Err("integer exceeds JavaScript's safe integer range")
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for JsSafeU64 {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for JsSafeU64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for JsSafeU64 {
    fn schema_name() -> String {
        "JsSafeU64".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_SAFE_INTEGER,
        }))
        .unwrap()
    }
}


/// The harnesses Bridge ships a hand-written adapter for today.
///
/// This is **not** a closed set of agents — it is the set Bridge currently has
/// bespoke code for. An id outside it is an ordinary agent Bridge runs through
/// a generic transport, and an id can move into this list later without its
/// sessions changing identity.
pub const BUILTIN_HARNESS_IDS: [&str; 5] = ["claude", "codex", "cursor", "opencode", "shell"];

/// Upper bound on a harness id. The live ACP registry's longest is 18
/// characters (`github-copilot-cli`); 64 leaves room without letting an
/// arbitrary string become a session's harness.
const MAX_HARNESS_ID: usize = 64;

/// The regular expression published in the JSON Schema. Kept beside
/// [`HarnessId::parse`], which is the enforcing implementation — the pattern
/// documents the grammar for generated clients, it does not define it.
const HARNESS_ID_PATTERN: &str = r"^[a-z0-9][a-z0-9._-]{0,63}$";

/// Why a harness id was refused. Carries the offending value so a client is
/// told what it actually sent, bounded so an oversized payload cannot inflate
/// the error it provokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessIdError {
    value: String,
    reason: &'static str,
}

impl HarnessIdError {
    fn new(value: &str, reason: &'static str) -> Self {
        const MAX_ECHO: usize = 80;
        let mut echoed: String = value.chars().take(MAX_ECHO).collect();
        if echoed.chars().count() < value.chars().count() {
            echoed.push('…');
        }
        Self { value: echoed, reason }
    }
}

impl std::fmt::Display for HarnessIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid harness id {:?}: {}", self.value, self.reason)
    }
}

impl std::error::Error for HarnessIdError {}

/// A harness identifier: **which agent**, never how Bridge runs it.
///
/// ```text
/// HarnessId := [a-z0-9][a-z0-9._-]{0,63}
/// ```
///
/// **Open on purpose.** This was a closed enum of four built-ins through
/// protocol 0.8. A marketplace installs agents that did not exist when a
/// client was compiled, so a closed set is the one shape that cannot work.
///
/// **One id per agent, not per integration.** Claude reached through the
/// Agent SDK sidecar and Claude reached through an ACP shim are the same
/// agent, so they share the id `claude`; which path served a session is
/// recorded separately and is Bridge's problem, not the user's. An earlier
/// draft namespaced registry-installed agents as `acp:<id>`, which leaked the
/// transport into identity: it made one agent look like two competing
/// products, and it would have forced a migration the first time a bespoke
/// adapter replaced a generic one. Adding a hand-written adapter for `gemini`
/// changes how it runs, never what it is called, and never breaks the sessions
/// it already owns.
///
/// Every value protocol 0.8 could carry parses here unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HarnessId(String);

impl HarnessId {
    /// Accepts the charset the ACP registry actually uses: ASCII lowercase
    /// alphanumerics, `-`, `.`, and `_`, opening on an alphanumeric. Every one
    /// of the 38 live entries satisfies it, and so do all four built-ins.
    pub fn parse(value: &str) -> Result<Self, HarnessIdError> {
        if value.is_empty() {
            return Err(HarnessIdError::new(value, "a harness id cannot be empty"));
        }
        if value.len() > MAX_HARNESS_ID {
            return Err(HarnessIdError::new(
                value,
                "a harness id may not exceed 64 characters",
            ));
        }
        if !value.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(HarnessIdError::new(
                value,
                "a harness id must start with a lowercase letter or digit",
            ));
        }
        if !value
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-' | '.' | '_'))
        {
            return Err(HarnessIdError::new(
                value,
                "a harness id may only contain lowercase letters, digits, '-', '.', and '_'",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// The canonical string: the wire value, the `sessions.harness` column
    /// value, and the adapter-registry key. There is no second spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether Bridge ships a hand-written adapter for this agent *today*.
    /// A presentation and capability hint — never part of its identity.
    pub fn is_builtin(&self) -> bool {
        BUILTIN_HARNESS_IDS.contains(&self.0.as_str())
    }
}

impl std::fmt::Display for HarnessId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HarnessId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for HarnessId {
    fn schema_name() -> String {
        "HarnessId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": HARNESS_ID_PATTERN,
            "description": "Which agent runs a session, e.g. 'claude', 'codex', 'gemini'. \
                            Identifies the agent only — how Bridge runs it is not encoded here.",
        }))
        .unwrap()
    }
}


/// A harness id as it appears in a **result**.
///
/// Deliberately *not* [`HarnessId`]. A stored session can carry an id this
/// server cannot interpret — a row written by a newer Bridge, or an agent
/// since uninstalled — and such a session must still list and replay under its
/// own name rather than disappear. A result therefore cannot promise the
/// [`HarnessId`] grammar, and **its schema must not claim to**: publishing the
/// strict pattern here would let `state/get_state` return a document that
/// fails its own contract.
///
/// Parameters keep the strict type. The asymmetry is the point — an id that
/// cannot be acted on has no business being sent back as a request, but it
/// must still be readable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoredHarnessId(String);

impl StoredHarnessId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The strict id, when this stored value satisfies the grammar. `None`
    /// means the harness cannot be acted on — not that the session is invalid.
    pub fn interpreted(&self) -> Option<HarnessId> {
        HarnessId::parse(&self.0).ok()
    }
}

impl From<HarnessId> for StoredHarnessId {
    fn from(id: HarnessId) -> Self {
        Self(id.0)
    }
}

impl std::fmt::Display for StoredHarnessId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl JsonSchema for StoredHarnessId {
    fn schema_name() -> String {
        "StoredHarnessId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "description": "The harness a session runs under. Usually a HarnessId, but \
                            unconstrained on purpose: a session persisted by a newer Bridge, \
                            or one whose agent was uninstalled, still reports the id it was \
                            stored with so its history stays readable. Such an id cannot be \
                            sent back as a parameter.",
        }))
        .unwrap()
    }
}

/// The public identity of an agent — the name a user picks, the name history is
/// filed under, and the only one of the four marketplace identities a user ever
/// sees.
///
/// An alias of [`HarnessId`] rather than a rename. `HarnessId` is the protocol
/// 0.8 wire spelling and already says exactly this in its own documentation:
/// **which agent**, never how Bridge runs it. Renaming it would churn every
/// message and break minor-version compatibility to change a word. `AgentId` is
/// the vocabulary the marketplace work reads in; they are the same type on
/// purpose, and the separation that matters — agent versus implementation — is
/// carried by [`BackendId`], which is genuinely distinct.
pub type AgentId = HarnessId;

/// Why a backend identity was refused. Carries the offending value so a caller
/// is told what it actually sent, bounded so an oversized value cannot inflate
/// the error it provokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityError {
    kind: &'static str,
    value: String,
    reason: &'static str,
}

impl IdentityError {
    fn new(kind: &'static str, value: &str, reason: &'static str) -> Self {
        const MAX_ECHO: usize = 80;
        let mut echoed: String = value.chars().take(MAX_ECHO).collect();
        if echoed.chars().count() < value.chars().count() {
            echoed.push('…');
        }
        Self { kind, value: echoed, reason }
    }

    /// Which identity was refused — `"backend id"`, `"backend version"`, or
    /// `"installation id"`. Lets a caller branch without matching the message.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid {} {:?}: {}",
            self.kind, self.value, self.reason
        )
    }
}

impl std::error::Error for IdentityError {}

/// Upper bound on a backend id, matching [`HarnessId`]'s so the two grammars
/// stay one rule rather than two that drift.
const MAX_BACKEND_ID: usize = MAX_HARNESS_ID;

const BACKEND_ID_PATTERN: &str = HARNESS_ID_PATTERN;

/// **How** Bridge reaches an agent: which implementation served a session.
///
/// ```text
/// BackendId := [a-z0-9][a-z0-9._-]{0,63}
/// ```
///
/// The counterpart to [`AgentId`], and deliberately a distinct type: one public
/// agent may have several implementations over its life — an official SDK today,
/// an ACP server tomorrow — and a session that resumes into a different one is
/// not the same session continuing. `AgentId` answers *what the user picked*;
/// `BackendId` answers *what actually ran*, which is Bridge's problem to record
/// and never the user's to name.
///
/// The convention is `<agent>.<transport>` — `claude.agent-sdk`,
/// `codex.app-server`, `opencode.server` — but it is a convention, not a rule.
/// Nothing here parses the parts, because a shared driver serving two agents
/// under one id is a shape this must not forbid in advance.
///
/// Both identities are lowercase dotted strings under the same grammar, so the
/// type system is the only thing that stops one being passed where the other
/// belongs. These must not compile:
///
/// ```compile_fail
/// use bridge_protocol::messages::{AgentId, BackendId};
/// fn takes_an_agent(_: AgentId) {}
/// takes_an_agent(BackendId::parse("claude.agent-sdk").unwrap());
/// ```
///
/// ```compile_fail
/// use bridge_protocol::messages::{AgentId, BackendId};
/// fn takes_a_backend(_: BackendId) {}
/// takes_a_backend(AgentId::parse("claude").unwrap());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BackendId(String);

impl BackendId {
    /// Accepts exactly [`HarnessId`]'s charset: ASCII lowercase alphanumerics,
    /// `-`, `.`, and `_`, opening on an alphanumeric.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        const KIND: &str = "backend id";
        if value.is_empty() {
            return Err(IdentityError::new(KIND, value, "a backend id cannot be empty"));
        }
        if value.len() > MAX_BACKEND_ID {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend id may not exceed 64 characters",
            ));
        }
        if !value.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend id must start with a lowercase letter or digit",
            ));
        }
        if !value
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-' | '.' | '_'))
        {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend id may only contain lowercase letters, digits, '-', '.', and '_'",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BackendId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BackendId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for BackendId {
    fn schema_name() -> String {
        "BackendId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": BACKEND_ID_PATTERN,
            "description": "Which implementation serves an agent, e.g. 'claude.agent-sdk', \
                            'codex.app-server'. Bridge's record of how a session was run — \
                            never the agent's public identity.",
        }))
        .unwrap()
    }
}

/// Upper bound on a backend version. Long enough for the longest real spelling
/// Bridge records — an npm coordinate with a scoped package name — and short
/// enough that an arbitrary string cannot become one.
const MAX_BACKEND_VERSION: usize = 64;

/// The concrete version of the implementation that served a session.
///
/// **Not semver, and not parsed.** The three proven integrations already report
/// `0.3.209`, `0.147.0`, and `1.18.16`, and a managed recipe pins a version as
/// `npm:@anthropic-ai/claude-agent-sdk@0.3.209`. Imposing a version *grammar*
/// here would be Bridge inventing a versioning policy for software it does not
/// publish; the only rules are the ones storage and display actually need — a
/// bounded, single-line, printable value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BackendVersion(String);

impl BackendVersion {
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        const KIND: &str = "backend version";
        if value.is_empty() {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend version cannot be empty",
            ));
        }
        if value.len() > MAX_BACKEND_VERSION {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend version may not exceed 64 characters",
            ));
        }
        if !value.chars().all(|character| character.is_ascii_graphic()) {
            return Err(IdentityError::new(
                KIND,
                value,
                "a backend version may only contain printable ASCII without spaces",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BackendVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BackendVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for BackendVersion {
    fn schema_name() -> String {
        "BackendVersion".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_BACKEND_VERSION,
            "description": "The version of the implementation that served a session, as the \
                            vendor spells it. Deliberately not semver-constrained.",
        }))
        .unwrap()
    }
}

/// How many characters `managed_payload::installation_id` produces: the leading
/// 24 of a SHA-256 hex digest over agent, version, platform, and integrity.
const INSTALLATION_ID_LENGTH: usize = 24;

/// Which managed payload a backend was launched from.
///
/// Present only when Bridge owns the copy that ran. An agent served by a runtime
/// on PATH has a [`BackendId`] and usually a [`BackendVersion`], but no
/// installation id — there is no receipt, because there is nothing Bridge
/// installed and nothing it may remove.
///
/// The grammar is exactly what the payload engine derives, so a value that did
/// not come from a receipt cannot be mistaken for one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct InstallationId(String);

impl InstallationId {
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        const KIND: &str = "installation id";
        if value.len() != INSTALLATION_ID_LENGTH {
            return Err(IdentityError::new(
                KIND,
                value,
                "an installation id is exactly 24 characters",
            ));
        }
        if !value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
        {
            return Err(IdentityError::new(
                KIND,
                value,
                "an installation id is lowercase hexadecimal",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InstallationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for InstallationId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for InstallationId {
    fn schema_name() -> String {
        "InstallationId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^[0-9a-f]{24}$",
            "description": "The managed payload a backend was launched from. Absent when the \
                            runtime is the user's own, because Bridge installed nothing.",
        }))
        .unwrap()
    }
}

/// A reasoning-effort level on the wire. Mirrors `bridge_core::delegation::Effort`
/// variant for variant; a mirror test in bridge-core keeps the two from drifting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
}

/// Successful result for commands that return no value. JSON-RPC carries Rust
/// unit as an explicit `null` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct UnitResult(pub ());

/// Serialize and deserialize a payload, so a test asserts on exactly what the
/// wire carries rather than on the Rust value it started from.
#[cfg(test)]
pub(crate) fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_results_are_explicit_json_null() {
        assert_eq!(serde_json::to_value(UnitResult(())).unwrap(), serde_json::Value::Null);
        assert_eq!(round_trip(&UnitResult(())), UnitResult(()));
    }

    /// The runtime half of the claim; the half that matters — that neither
    /// converts to the other — is two `compile_fail` doc tests on [`BackendId`],
    /// where rustdoc actually collects them.
    #[test]
    fn backend_id_and_agent_id_are_distinct_types() {
        // Same grammar, so only the types tell them apart.
        for value in ["claude", "codex.app-server", "a", "gemini-cli.acp"] {
            assert_eq!(
                BackendId::parse(value).unwrap().as_str(),
                AgentId::parse(value).unwrap().as_str(),
                "{value} must be legal under both grammars"
            );
        }
        for value in ["", "Claude", "-codex", "codex/app-server", "codex server"] {
            assert!(BackendId::parse(value).is_err(), "{value:?} must be refused");
            assert!(AgentId::parse(value).is_err(), "{value:?} must be refused");
        }
        assert!(BackendId::parse(&"a".repeat(65)).is_err());
        assert_eq!(BackendId::parse(&"a".repeat(64)).unwrap().as_str().len(), 64);

        let backend = BackendId::parse("claude.agent-sdk").unwrap();
        assert_eq!(round_trip(&backend), backend);
        assert_eq!(
            serde_json::to_value(&backend).unwrap(),
            serde_json::json!("claude.agent-sdk")
        );
        assert!(serde_json::from_value::<BackendId>(serde_json::json!("Claude")).is_err());
    }

    #[test]
    fn backend_version_accepts_real_vendor_spellings_and_bounds_the_rest() {
        // Every one of these is a version Bridge has actually recorded: the
        // three payloads #179 installed, and the npm coordinate a recipe pins.
        for value in [
            "0.3.209",
            "0.147.0",
            "1.18.16",
            "npm:@anthropic-ai/claude-agent-sdk@0.3.209",
            "2.0.0-beta.1+build.7",
        ] {
            assert_eq!(BackendVersion::parse(value).unwrap().as_str(), value);
        }
        for value in ["", "1.0.0 ", "1.0\n0", "1.0\t0", "1.0.0\u{0}"] {
            assert!(
                BackendVersion::parse(value).is_err(),
                "{value:?} must be refused"
            );
        }
        assert!(BackendVersion::parse(&"9".repeat(65)).is_err());
        assert!(BackendVersion::parse(&"9".repeat(64)).is_ok());

        let version = BackendVersion::parse("0.3.209").unwrap();
        assert_eq!(round_trip(&version), version);
        assert_eq!(serde_json::to_value(&version).unwrap(), serde_json::json!("0.3.209"));
    }

    #[test]
    fn installation_id_matches_what_the_payload_engine_derives() {
        // 24 lowercase hex characters — the leading half of a SHA-256 digest,
        // exactly as `managed_payload::installation_id` formats it. A core-side
        // test asserts a real receipt's id parses here.
        let derived = "3f9a0c1b7e2d4856af01bc93";
        assert_eq!(derived.len(), INSTALLATION_ID_LENGTH);
        let id = InstallationId::parse(derived).unwrap();
        assert_eq!(id.as_str(), derived);
        assert_eq!(round_trip(&id), id);

        for wrong in [
            "",
            "3f9a0c1b7e2d4856af01bc9",   // 23
            "3f9a0c1b7e2d4856af01bc934", // 25
            "3F9A0C1B7E2D4856AF01BC93",  // uppercase
            "3f9a0c1b7e2d4856af01bcg3",  // not hex
        ] {
            assert!(
                InstallationId::parse(wrong).is_err(),
                "{wrong:?} must be refused"
            );
        }
    }

    #[test]
    fn an_identity_error_is_branchable_without_matching_its_message() {
        let backend = BackendId::parse("Claude").unwrap_err();
        let version = BackendVersion::parse("").unwrap_err();
        let installation = InstallationId::parse("nope").unwrap_err();
        assert_eq!(backend.kind(), "backend id");
        assert_eq!(version.kind(), "backend version");
        assert_eq!(installation.kind(), "installation id");
        assert!(backend.to_string().starts_with("invalid backend id"), "{backend}");

        // Bounded echo: an oversized value cannot inflate the error it provokes.
        let huge = "Z".repeat(5_000);
        let error = BackendId::parse(&huge).unwrap_err().to_string();
        assert!(error.len() < 200, "error echoed {} bytes", error.len());
    }

    #[test]
    fn no_identity_type_can_carry_a_credential() {
        // A credential is not a printable-ASCII-and-short problem; the reason
        // these types are safe is that they are opaque, validated scalars with
        // no free-text field. Pin the shape so a later field addition has to
        // come past this test.
        let backend = serde_json::to_value(BackendId::parse("codex.app-server").unwrap()).unwrap();
        let version = serde_json::to_value(BackendVersion::parse("0.147.0").unwrap()).unwrap();
        let installation =
            serde_json::to_value(InstallationId::parse("3f9a0c1b7e2d4856af01bc93").unwrap())
                .unwrap();
        for value in [&backend, &version, &installation] {
            assert!(value.is_string(), "identities are scalars, not objects: {value}");
        }
    }

    #[test]
    fn effort_levels_are_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_value(Effort::Xhigh).unwrap(), serde_json::json!("xhigh"));
        assert_eq!(round_trip(&Effort::Medium), Effort::Medium);
        assert!(serde_json::from_value::<Effort>(serde_json::json!("Xhigh")).is_err());
        assert!(serde_json::from_value::<Effort>(serde_json::json!("extreme")).is_err());
    }

    #[test]
    fn harness_ids_name_agents_including_ones_this_build_never_heard_of() {
        for builtin in BUILTIN_HARNESS_IDS {
            let id = HarnessId::parse(builtin).unwrap();
            assert!(id.is_builtin(), "{builtin}");
            assert_eq!(id.as_str(), builtin);
            assert_eq!(round_trip(&id), id);
        }
        for agent in ["gemini", "github-copilot-cli", "mistral-vibe", "pi-acp", "vtcode"] {
            let id = HarnessId::parse(agent).unwrap();
            assert!(!id.is_builtin(), "{agent}");
            assert_eq!(id.as_str(), agent);
            assert_eq!(round_trip(&id), id);
        }
    }

    #[test]
    fn builtin_harness_ids_serialize_exactly_as_protocol_0_8() {
        // These strings are persisted in the `sessions.harness` column and
        // keyed on in the adapter registry. Opening the identifier must not
        // have moved one, and adding a bespoke adapter only ever appends.
        assert_eq!(
            BUILTIN_HARNESS_IDS,
            ["claude", "codex", "cursor", "opencode", "shell"]
        );
        for builtin in BUILTIN_HARNESS_IDS {
            let id = HarnessId::parse(builtin).unwrap();
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(builtin));
        }
    }

    #[test]
    fn an_agent_id_does_not_encode_how_bridge_runs_it() {
        // One id per agent. The registry publishes an `opencode` entry and
        // Bridge ships an OpenCode adapter; they are the same agent reached
        // two ways, so they share one id and one session history. An earlier
        // draft spelled the registry-installed one `acp:opencode`, which made
        // one agent look like two products — that spelling is now invalid.
        let opencode = HarnessId::parse("opencode").unwrap();
        assert!(opencode.is_builtin());
        assert!(HarnessId::parse("acp:opencode").is_err());
        assert!(HarnessId::parse("acp:gemini").is_err());

        // `is_builtin` is a capability hint, never identity: the day Bridge
        // ships a bespoke Gemini adapter, the id is unchanged.
        let gemini = HarnessId::parse("gemini").unwrap();
        assert!(!gemini.is_builtin());
        assert_eq!(gemini.as_str(), "gemini");
    }

    #[test]
    fn harness_ids_reject_malformed_values() {
        let rejected = [
            "",                 // empty
            "Claude",           // ids are lowercase
            "-gemini",          // must open on an alphanumeric
            ".gemini",          //  "
            "gem ini",          // no whitespace
            "gem/ini",          // no path separators
            "acp:gemini",       // ':' is not in the charset
            " claude",          // not trimmed for the caller
            "claude ",          //  "
            "shell\n",          //  "
        ];
        for value in rejected {
            assert!(HarnessId::parse(value).is_err(), "{value:?} should not be a harness id");
            assert!(
                serde_json::from_value::<HarnessId>(serde_json::json!(value)).is_err(),
                "{value:?} should not deserialize"
            );
        }
        assert!(HarnessId::parse(&"a".repeat(MAX_HARNESS_ID)).is_ok());
        assert!(HarnessId::parse(&"a".repeat(MAX_HARNESS_ID + 1)).is_err());
    }

    #[test]
    fn rejection_names_the_value_without_echoing_an_unbounded_one() {
        let error = HarnessId::parse("Claude").unwrap_err();
        assert!(error.to_string().contains("Claude"), "{error}");

        let huge = "z".repeat(10_000);
        let message = HarnessId::parse(&huge).unwrap_err().to_string();
        assert!(message.len() < 200, "error echoed an unbounded value: {}", message.len());
        assert!(message.contains('…'), "{message}");
    }

    #[test]
    fn harness_id_schema_publishes_the_grammar_as_a_string() {
        let schema = serde_json::to_value(schemars::schema_for!(HarnessId)).unwrap();
        assert_eq!(schema["type"], serde_json::json!("string"));
        assert_eq!(schema["pattern"], serde_json::json!(HARNESS_ID_PATTERN));
        // No `enum` key: a generated client must not close the set over the
        // values that happen to exist today.
        assert!(schema.get("enum").is_none(), "{schema}");
    }

    #[test]
    fn stored_harness_ids_accept_what_parameters_refuse() {
        // The result type must take anything the `sessions.harness` column can
        // hold, or `state/get_state` returns a document failing its own
        // schema. Its schema must publish no `pattern` for the same reason.
        for stored in ["claude", "gemini", "acp:gemini", "", "shel1", "Claude"] {
            let id = serde_json::from_value::<StoredHarnessId>(serde_json::json!(stored))
                .unwrap_or_else(|error| panic!("{stored:?} must be readable: {error}"));
            assert_eq!(id.as_str(), stored);
            assert_eq!(round_trip(&id), id);
            // Actionable exactly when it satisfies the parameter grammar.
            assert_eq!(id.interpreted().is_some(), HarnessId::parse(stored).is_ok(), "{stored:?}");
            // ...and the strict type refuses precisely the difference.
            if id.interpreted().is_none() {
                assert!(serde_json::from_value::<HarnessId>(serde_json::json!(stored)).is_err());
            }
        }

        let schema = serde_json::to_value(schemars::schema_for!(StoredHarnessId)).unwrap();
        assert_eq!(schema["type"], serde_json::json!("string"));
        assert!(
            schema.get("pattern").is_none(),
            "a result must not publish a grammar it cannot keep: {schema}"
        );
        assert!(schema.get("enum").is_none(), "{schema}");
    }

    #[test]
    fn javascript_safe_integers_enforce_both_boundaries() {
        for value in [-MAX_SAFE_INTEGER, 0, MAX_SAFE_INTEGER] {
            let safe = serde_json::from_value::<JsSafeI64>(serde_json::json!(value)).unwrap();
            assert_eq!(safe.get(), value);
        }
        for value in [0, MAX_SAFE_INTEGER as u64] {
            let safe = serde_json::from_value::<JsSafeU64>(serde_json::json!(value)).unwrap();
            assert_eq!(safe.get(), value);
        }
        for value in [MAX_SAFE_INTEGER + 1, -(MAX_SAFE_INTEGER + 1)] {
            let error = serde_json::from_value::<JsSafeI64>(serde_json::json!(value)).unwrap_err();
            assert!(error.to_string().contains("safe integer"), "{error}");
        }
        let error = serde_json::from_value::<JsSafeU64>(serde_json::json!(
            MAX_SAFE_INTEGER as u64 + 1
        ))
        .unwrap_err();
        assert!(error.to_string().contains("safe integer"), "{error}");
        assert!(serde_json::from_value::<JsSafeU64>(serde_json::json!(-1)).is_err());

        let signed_schema = serde_json::to_value(schemars::schema_for!(JsSafeI64)).unwrap();
        assert_eq!(signed_schema["minimum"].as_f64(), Some(-(MAX_SAFE_INTEGER as f64)));
        assert_eq!(signed_schema["maximum"].as_f64(), Some(MAX_SAFE_INTEGER as f64));
        let unsigned_schema = serde_json::to_value(schemars::schema_for!(JsSafeU64)).unwrap();
        assert_eq!(unsigned_schema["minimum"].as_f64(), Some(0.0));
        assert_eq!(unsigned_schema["maximum"].as_f64(), Some(MAX_SAFE_INTEGER as f64));
    }
}
