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

/// The harnesses Bridge implements itself, each with a hand-written adapter.
/// The bare namespace on the wire holds exactly these and nothing else.
pub const BUILTIN_HARNESS_IDS: [&str; 4] = ["claude", "codex", "opencode", "shell"];

/// Namespace separating a registry-installed ACP agent from a built-in.
pub const ACP_HARNESS_PREFIX: &str = "acp:";

/// Upper bound on a registry agent id. The live catalog's longest is 18
/// characters (`github-copilot-cli`); 64 leaves room for growth without
/// letting an arbitrary string become a session's harness.
const MAX_ACP_AGENT_ID: usize = 64;

/// The regular expression published in the JSON Schema. Kept beside
/// [`AcpAgentId::parse`], which is the enforcing implementation — the pattern
/// documents the grammar for generated clients, it does not define it.
const HARNESS_ID_PATTERN: &str = r"^(claude|codex|opencode|shell|acp:[a-z0-9][a-z0-9._-]{0,63})$";

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

/// The bare id of an ACP agent as the registry publishes it, e.g. `gemini`.
///
/// Validated on construction so an id that reached a session, an install
/// record, or an adapter-registry key is known to be well-formed. Defined here
/// rather than in bridge-core so the wire contract and the runtime share one
/// definition of what an agent may be called.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AcpAgentId(String);

impl AcpAgentId {
    /// Accepts the charset the registry actually uses: ASCII lowercase
    /// alphanumerics, `-`, `.`, and `_`, opening on an alphanumeric. Every one
    /// of the 38 live entries satisfies this; a test pins that against the
    /// captured index so upstream widening its ids is a test failure rather
    /// than a catalog entry Bridge cannot name.
    pub fn parse(value: &str) -> Result<Self, HarnessIdError> {
        if value.is_empty() {
            return Err(HarnessIdError::new(value, "an agent id cannot be empty"));
        }
        if value.len() > MAX_ACP_AGENT_ID {
            return Err(HarnessIdError::new(
                value,
                "an agent id may not exceed 64 characters",
            ));
        }
        if !value.starts_with(|first: char| first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(HarnessIdError::new(
                value,
                "an agent id must start with a lowercase letter or digit",
            ));
        }
        if !value
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-' | '.' | '_'))
        {
            return Err(HarnessIdError::new(
                value,
                "an agent id may only contain lowercase letters, digits, '-', '.', and '_'",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AcpAgentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AcpAgentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for AcpAgentId {
    fn schema_name() -> String {
        "AcpAgentId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": r"^[a-z0-9][a-z0-9._-]{0,63}$",
            "description": "The bare id of an ACP agent as its registry entry publishes it.",
        }))
        .unwrap()
    }
}

/// A harness identifier on the wire.
///
/// Exactly two shapes, and no third:
///
/// ```text
/// HarnessId := "claude" | "codex" | "opencode" | "shell"   (built-in)
///            | "acp:" <AcpAgentId>                         (ACP agent)
/// ```
///
/// **Open on purpose.** This was a closed enum through protocol 0.8, mirroring
/// `bridge_core::model::Harness` variant for variant. A marketplace installs
/// agents that did not exist when a client was compiled, so a closed set is
/// the one shape that cannot work — the exhaustive `match` that is a feature
/// elsewhere in this codebase is precisely what a catalog of agents breaks.
/// Core keeps its enum (built-ins have bespoke behaviour and deserve
/// compile-time exhaustiveness); the wire does not.
///
/// **The bare namespace is reserved for built-ins.** A bare id that is not a
/// built-in is rejected rather than read as an agent, because otherwise
/// `gemini` is permanently ambiguous: a future built-in, or an agent installed
/// today. The `acp:` prefix is not decoration — the live registry publishes an
/// entry whose id is `opencode`, colliding head-on with Bridge's built-in
/// OpenCode adapter. Prefixing makes that collision unrepresentable instead of
/// merely detected.
///
/// Every value protocol 0.8 could carry serializes byte-identically here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HarnessId(String);

impl HarnessId {
    pub fn parse(value: &str) -> Result<Self, HarnessIdError> {
        if BUILTIN_HARNESS_IDS.contains(&value) {
            return Ok(Self(value.to_owned()));
        }
        match value.strip_prefix(ACP_HARNESS_PREFIX) {
            Some(agent) => AcpAgentId::parse(agent).map(Self::from),
            None => Err(HarnessIdError::new(
                value,
                "expected a built-in harness or an 'acp:'-prefixed agent id",
            )),
        }
    }

    /// The canonical string: the wire value, the `sessions.harness` column
    /// value, and the adapter-registry key. There is no second spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_builtin(&self) -> bool {
        BUILTIN_HARNESS_IDS.contains(&self.0.as_str())
    }

    /// The bare agent id, for an ACP harness only.
    pub fn acp_agent_id(&self) -> Option<&str> {
        self.0.strip_prefix(ACP_HARNESS_PREFIX)
    }
}

impl From<AcpAgentId> for HarnessId {
    fn from(agent: AcpAgentId) -> Self {
        Self(format!("{ACP_HARNESS_PREFIX}{agent}"))
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
            "description": "A built-in harness ('claude', 'codex', 'opencode', 'shell') \
                            or an installed ACP agent ('acp:' + its registry id).",
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

    #[test]
    fn effort_levels_are_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_value(Effort::Xhigh).unwrap(), serde_json::json!("xhigh"));
        assert_eq!(round_trip(&Effort::Medium), Effort::Medium);
        assert!(serde_json::from_value::<Effort>(serde_json::json!("Xhigh")).is_err());
        assert!(serde_json::from_value::<Effort>(serde_json::json!("extreme")).is_err());
    }

    #[test]
    fn harness_ids_accept_builtins_and_acp_ids() {
        for builtin in BUILTIN_HARNESS_IDS {
            let id = HarnessId::parse(builtin).unwrap();
            assert!(id.is_builtin(), "{builtin}");
            assert_eq!(id.acp_agent_id(), None);
            assert_eq!(id.as_str(), builtin);
            assert_eq!(round_trip(&id), id);
        }
        let agent = HarnessId::parse("acp:github-copilot-cli").unwrap();
        assert!(!agent.is_builtin());
        assert_eq!(agent.acp_agent_id(), Some("github-copilot-cli"));
        assert_eq!(round_trip(&agent), agent);
    }

    #[test]
    fn builtin_harness_ids_serialize_exactly_as_protocol_0_8() {
        // Pinned against literals: these four strings are persisted in the
        // `sessions.harness` column and keyed on in the adapter registry, so a
        // refactor must never quietly rename one.
        assert_eq!(BUILTIN_HARNESS_IDS, ["claude", "codex", "opencode", "shell"]);
        for builtin in BUILTIN_HARNESS_IDS {
            let id = HarnessId::parse(builtin).unwrap();
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(builtin));
        }
        let agent = HarnessId::parse("acp:gemini").unwrap();
        assert_eq!(serde_json::to_value(&agent).unwrap(), serde_json::json!("acp:gemini"));
    }

    #[test]
    fn harness_ids_reject_malformed_values() {
        let rejected = [
            "",                          // empty
            "acp:",                      // prefix with no agent
            "acp:acp:gemini",            // nested prefix — ':' is not in the charset
            "gemini",                    // bare namespace is reserved for built-ins
            "Claude",                    // built-ins are lowercase
            "acp:Gemini",                // agent ids are lowercase
            "acp:-gemini",               // must open on an alphanumeric
            "acp:.gemini",               //  "
            "acp:gem ini",               // no whitespace
            "acp:gem/ini",               // no path separators
            "acp:gem:ini",               // no colons
            " claude",                   // not trimmed for the caller
            "claude ",                   //  "
            "shell\n",                   //  "
        ];
        for value in rejected {
            assert!(
                HarnessId::parse(value).is_err(),
                "{value:?} should not be a harness id"
            );
            assert!(
                serde_json::from_value::<HarnessId>(serde_json::json!(value)).is_err(),
                "{value:?} should not deserialize"
            );
        }
        let too_long = format!("acp:{}", "a".repeat(MAX_ACP_AGENT_ID + 1));
        assert!(HarnessId::parse(&too_long).is_err());
        assert!(HarnessId::parse(&format!("acp:{}", "a".repeat(MAX_ACP_AGENT_ID))).is_ok());
    }

    #[test]
    fn rejection_names_the_value_without_echoing_an_unbounded_one() {
        let error = HarnessId::parse("gemini").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("gemini"), "{message}");
        assert!(message.contains("acp:"), "{message}");

        let huge = "z".repeat(10_000);
        let message = AcpAgentId::parse(&huge).unwrap_err().to_string();
        assert!(message.len() < 200, "error echoed an unbounded value: {}", message.len());
        assert!(message.contains('…'), "{message}");
    }

    #[test]
    fn acp_prefix_makes_builtin_collision_unrepresentable() {
        // The live registry ships an entry with id `opencode`, which collides
        // with Bridge's own OpenCode adapter. The namespaces keep both nameable
        // and never equal.
        let builtin = HarnessId::parse("opencode").unwrap();
        let from_registry = HarnessId::from(AcpAgentId::parse("opencode").unwrap());
        assert_eq!(from_registry.as_str(), "acp:opencode");
        assert_ne!(builtin, from_registry);
        assert!(builtin.is_builtin());
        assert!(!from_registry.is_builtin());
        assert_eq!(from_registry.acp_agent_id(), Some("opencode"));
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
