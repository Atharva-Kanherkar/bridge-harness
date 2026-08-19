//! Which connectors a briefing may read, and how their results become evidence.
//!
//! Slice 2 decided *whether* a tool may be called. This decides whether a tool is
//! worth offering in the first place, which is a different and stricter question. A
//! connector reaches a briefing only when three things are true at once:
//!
//! 1. Bridge holds a **reviewed exact tool identity** for it — someone looked at that
//!    tool and said this one, not the family it belongs to.
//! 2. The tool's **definition digest still matches** what was reviewed. A tool whose
//!    schema changed is a tool nobody has reviewed, whatever its name still says.
//! 3. Its family has a **deterministic evidence resolver** — something that can turn a
//!    result into a canonical resource id and a safe target without asking the model.
//!
//! The third is the one that keeps provenance honest. If Bridge cannot say where a
//! result came from on its own, the only other candidate is the model, and a model
//! that authors its own citations can cite anything.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::briefing_policy::BriefingToolIdentity;

/// A connector family Bridge knows how to derive provenance for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorFamily {
    Slack,
    Gmail,
    GitHub,
    Linear,
    Notion,
}

impl ConnectorFamily {
    pub const ALL: [Self; 5] = [Self::Slack, Self::Gmail, Self::GitHub, Self::Linear, Self::Notion];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Gmail => "gmail",
            Self::GitHub => "github",
            Self::Linear => "linear",
            Self::Notion => "notion",
        }
    }

    /// Hosts a permalink from this family may point at. An external target is only
    /// ever produced when Bridge resolved it *and* its host is on this list — the
    /// allowlist is what stops a result from turning into a link somewhere else.
    pub fn allowed_hosts(self) -> &'static [&'static str] {
        match self {
            Self::Slack => &["slack.com", "app.slack.com"],
            Self::Gmail => &["mail.google.com"],
            Self::GitHub => &["github.com"],
            Self::Linear => &["linear.app"],
            Self::Notion => &["notion.so", "www.notion.so"],
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.as_str() == value)
    }
}

/// Where a piece of evidence can be opened. Bridge-derived in every variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum EvidenceTarget {
    /// A permalink Bridge resolved and matched against the family's allowlist.
    ExternalLink { url: String, host: String },
    /// A local Bridge session. Needs no allowlist: it is not a link out.
    Session { session_id: String },
    /// Nothing safe to open. Not an error — plenty of evidence has no permalink.
    None,
}

/// Why a connector instance is not being offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum IneligibleReason {
    /// Bridge has no resolver for this family, so it could not derive provenance.
    NoResolver { family: String },
    /// Nobody reviewed a tool on this instance.
    NoReviewedTools,
    /// A reviewed tool's definition changed since it was reviewed.
    ToolDefinitionChanged { tool: String },
    /// The connector needs the user to sign in again.
    AuthRequired,
}

impl IneligibleReason {
    pub fn reason(&self) -> String {
        match self {
            Self::NoResolver { family } => format!(
                "Bridge cannot derive provenance for `{family}` results, so it will not offer them to a model"
            ),
            Self::NoReviewedTools => {
                "no tool on this connector has been reviewed for briefing".into()
            }
            Self::ToolDefinitionChanged { tool } => format!(
                "`{tool}` changed since it was reviewed, so it is not a reviewed tool any more"
            ),
            Self::AuthRequired => "this connector needs to be signed in again".into(),
        }
    }
}

/// One tool somebody reviewed, pinned to the definition they reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedTool {
    pub identity: BriefingToolIdentity,
    /// Digest of the tool definition at review time. The pin that makes "reviewed"
    /// mean something after the provider ships a change.
    pub definition_digest: String,
}

/// Digest a tool definition the way review does, so the two are comparable.
///
/// Over the canonical JSON of the definition rather than a display string: a schema
/// change that does not alter the name is exactly the change worth catching.
pub fn tool_definition_digest(definition: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"bridge-tool-definition-v1\0");
    hasher.update(canonical_json(definition).as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Serialize with object keys sorted, so two equal definitions digest equally
/// regardless of the order a provider happened to emit them in.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<&String, &serde_json::Value> = map.iter().collect();
            let inner: Vec<String> = sorted
                .into_iter()
                .map(|(key, item)| format!("{}:{}", serde_json::to_string(key).unwrap_or_default(), canonical_json(item)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// A connector instance as Bridge currently sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorInstance {
    pub instance_id: String,
    pub family: Option<ConnectorFamily>,
    /// The account this instance is signed in as. Part of provenance: the same tool on
    /// two accounts reads two different things.
    pub account_identity: Option<String>,
    pub authenticated: bool,
    pub reviewed_tools: Vec<ReviewedTool>,
    /// What the provider presents right now, tool name to its definition digest.
    pub presented_digests: BTreeMap<String, String>,
}

/// What a briefing is allowed to do with one connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorEligibility {
    Eligible {
        family: ConnectorFamily,
        /// Only the tools whose definitions still match their review.
        tools: Vec<BriefingToolIdentity>,
    },
    Ineligible(IneligibleReason),
}

impl ConnectorEligibility {
    pub fn is_eligible(&self) -> bool {
        matches!(self, Self::Eligible { .. })
    }
}

/// Decide what one connector instance may contribute.
///
/// Order matters: authentication is checked before tools, because an instance nobody
/// is signed into has nothing to offer regardless of what was reviewed, and a reader
/// deserves the actionable reason rather than a downstream one.
pub fn eligibility(instance: &ConnectorInstance) -> ConnectorEligibility {
    let Some(family) = instance.family else {
        return ConnectorEligibility::Ineligible(IneligibleReason::NoResolver {
            family: "unknown".into(),
        });
    };
    if !instance.authenticated {
        return ConnectorEligibility::Ineligible(IneligibleReason::AuthRequired);
    }
    if instance.reviewed_tools.is_empty() {
        return ConnectorEligibility::Ineligible(IneligibleReason::NoReviewedTools);
    }

    let mut tools = Vec::new();
    for reviewed in &instance.reviewed_tools {
        let wire_name = reviewed.identity.wire_name();
        match instance.presented_digests.get(&wire_name) {
            // The digest is the whole point: a name that still matches while the
            // definition underneath it changed is the case a name check misses.
            Some(current) if *current == reviewed.definition_digest => {
                tools.push(reviewed.identity.clone())
            }
            Some(_) => {
                return ConnectorEligibility::Ineligible(IneligibleReason::ToolDefinitionChanged {
                    tool: wire_name,
                })
            }
            // A reviewed tool the provider no longer presents cannot be called, and
            // silently dropping it would offer a narrower run than was reviewed
            // without anybody noticing.
            None => {
                return ConnectorEligibility::Ineligible(IneligibleReason::ToolDefinitionChanged {
                    tool: wire_name,
                })
            }
        }
    }
    // Every reviewed tool either matched its digest and was pushed, or returned above,
    // so a non-empty review list yields a non-empty tool list here.
    ConnectorEligibility::Eligible { family, tools }
}

/// Bridge's own reading of one successful connector result.
///
/// Everything here is derived. The model contributes nothing to it, which is what
/// makes it usable as provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEvidence {
    /// Stable across runs for the same underlying thing, so slice 5 can tell a task it
    /// has seen before from a new one.
    pub canonical_resource_id: String,
    pub source_kind: String,
    pub target: EvidenceTarget,
}

/// Turn one successful result into provenance, or refuse to.
///
/// `None` means Bridge could not derive a canonical id, and evidence without one is
/// evidence that cannot be recognised again — so it is not recorded at all rather than
/// recorded with something guessed.
pub fn resolve_evidence(
    family: ConnectorFamily,
    instance_id: &str,
    result: &serde_json::Value,
) -> Option<ResolvedEvidence> {
    let text = |key: &str| result.get(key).and_then(serde_json::Value::as_str).map(str::trim).filter(|value| !value.is_empty());
    let (source_kind, native_id) = match family {
        ConnectorFamily::Slack => ("slack.message", text("ts").or_else(|| text("messageTs"))?),
        ConnectorFamily::Gmail => ("gmail.thread", text("threadId").or_else(|| text("id"))?),
        ConnectorFamily::GitHub => ("github.item", text("nodeId").or_else(|| text("id"))?),
        ConnectorFamily::Linear => ("linear.issue", text("identifier").or_else(|| text("id"))?),
        ConnectorFamily::Notion => ("notion.page", text("pageId").or_else(|| text("id"))?),
    };
    Some(ResolvedEvidence {
        // Namespaced by instance, so the same native id on two accounts is two things.
        canonical_resource_id: format!("{}:{}:{}", family.as_str(), instance_id, native_id),
        source_kind: source_kind.to_owned(),
        target: resolve_target(family, result),
    })
}

/// A permalink, but only one Bridge resolved and matched against the family's hosts.
///
/// A model-authored URL never reaches here — this reads the connector's own result —
/// but the connector's result is untrusted too, so the host check applies to it just
/// the same.
pub fn resolve_target(family: ConnectorFamily, result: &serde_json::Value) -> EvidenceTarget {
    let candidate = ["permalink", "url", "htmlUrl", "webUrl"]
        .into_iter()
        .find_map(|key| result.get(key).and_then(serde_json::Value::as_str));
    let Some(url) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
        return EvidenceTarget::None;
    };
    match safe_external_link(family, url) {
        Some(target) => target,
        None => EvidenceTarget::None,
    }
}

/// Parse a URL and accept it only as https on an allowlisted host.
pub fn safe_external_link(family: ConnectorFamily, url: &str) -> Option<EvidenceTarget> {
    // Scheme first: a `javascript:` or `file:` target is not made safe by its host.
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Credentials in the authority would let `evil.example` hide behind a userinfo
    // segment that ends in an allowlisted name.
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = authority.split(':').next().unwrap_or(authority).to_ascii_lowercase();
    // Exact host match. A suffix test would admit `github.com.evil.example`.
    if !family.allowed_hosts().iter().any(|allowed| *allowed == host) {
        return None;
    }
    Some(EvidenceTarget::ExternalLink {
        url: url.to_owned(),
        host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity(server: &str, tool: &str) -> BriefingToolIdentity {
        BriefingToolIdentity { server: server.into(), tool: tool.into() }
    }

    fn definition() -> serde_json::Value {
        json!({"name": "search", "parameters": {"query": {"type": "string"}}})
    }

    /// A signed-in Slack instance with one reviewed tool whose digest still matches.
    fn instance() -> ConnectorInstance {
        let reviewed = identity("slack-1", "search_messages");
        let digest = tool_definition_digest(&definition());
        ConnectorInstance {
            instance_id: "slack-1".into(),
            family: Some(ConnectorFamily::Slack),
            account_identity: Some("T0001/U0001".into()),
            authenticated: true,
            reviewed_tools: vec![ReviewedTool { identity: reviewed.clone(), definition_digest: digest.clone() }],
            presented_digests: BTreeMap::from([(reviewed.wire_name(), digest)]),
        }
    }

    // -----------------------------------------------------------------------
    // What reaches the model
    // -----------------------------------------------------------------------

    #[test]
    fn only_reviewed_exact_tools_are_offered() {
        let mut subject = instance();
        // The provider presents a second tool nobody reviewed.
        subject.presented_digests.insert("mcp__slack-1__post_message".into(), "whatever".into());
        let ConnectorEligibility::Eligible { tools, family } = eligibility(&subject) else {
            panic!("the reviewed tool should make this eligible");
        };
        assert_eq!(family, ConnectorFamily::Slack);
        assert_eq!(
            tools.iter().map(BriefingToolIdentity::wire_name).collect::<Vec<_>>(),
            vec!["mcp__slack-1__search_messages"],
            "an unreviewed tool is not offered just because the provider has it"
        );
    }

    #[test]
    fn a_connector_without_a_resolver_is_ineligible() {
        // A family Bridge cannot derive provenance for is never offered, however many
        // tools it has: the only other candidate for authoring provenance is the model.
        let mut subject = instance();
        subject.family = None;
        let ConnectorEligibility::Ineligible(reason) = eligibility(&subject) else {
            panic!("expected ineligible");
        };
        assert!(matches!(reason, IneligibleReason::NoResolver { .. }));
        assert!(reason.reason().contains("cannot derive provenance"));
    }

    #[test]
    fn a_connector_with_no_reviewed_tools_is_ineligible() {
        let mut subject = instance();
        subject.reviewed_tools.clear();
        assert!(matches!(
            eligibility(&subject),
            ConnectorEligibility::Ineligible(IneligibleReason::NoReviewedTools)
        ));
    }

    #[test]
    fn an_unauthenticated_connector_reports_auth_rather_than_a_downstream_reason() {
        // Checked before tools: a reader gets the actionable reason, and signing in is
        // the action.
        let mut subject = instance();
        subject.authenticated = false;
        subject.reviewed_tools.clear();
        assert!(matches!(
            eligibility(&subject),
            ConnectorEligibility::Ineligible(IneligibleReason::AuthRequired)
        ));
    }

    #[test]
    fn a_changed_tool_definition_digest_makes_the_tool_ineligible() {
        // The case a name check misses entirely: same tool name, different schema.
        let mut subject = instance();
        let changed = tool_definition_digest(&json!({
            "name": "search",
            "parameters": {"query": {"type": "string"}, "andAlsoDelete": {"type": "boolean"}}
        }));
        subject.presented_digests.insert("mcp__slack-1__search_messages".into(), changed);
        let ConnectorEligibility::Ineligible(reason) = eligibility(&subject) else {
            panic!("a changed definition must not stay eligible");
        };
        assert_eq!(
            reason,
            IneligibleReason::ToolDefinitionChanged { tool: "mcp__slack-1__search_messages".into() }
        );
        assert!(reason.reason().contains("not a reviewed tool any more"));
    }

    #[test]
    fn a_reviewed_tool_the_provider_no_longer_presents_is_ineligible() {
        let mut subject = instance();
        subject.presented_digests.clear();
        assert!(matches!(
            eligibility(&subject),
            ConnectorEligibility::Ineligible(IneligibleReason::ToolDefinitionChanged { .. })
        ));
    }

    #[test]
    fn a_digest_ignores_key_order_but_not_content() {
        // Two providers may serialize the same definition differently; that must not
        // read as drift. A real change must.
        let one = tool_definition_digest(&json!({"a": 1, "b": {"c": 2, "d": 3}}));
        let other = tool_definition_digest(&json!({"b": {"d": 3, "c": 2}, "a": 1}));
        assert_eq!(one, other, "key order is not a change");
        assert_ne!(one, tool_definition_digest(&json!({"a": 1, "b": {"c": 2, "d": 4}})));
        assert_ne!(one, tool_definition_digest(&json!({"a": "1", "b": {"c": 2, "d": 3}})));
    }

    // -----------------------------------------------------------------------
    // Provenance Bridge derives
    // -----------------------------------------------------------------------

    #[test]
    fn the_canonical_resource_id_is_derived_not_taken() {
        let resolved = resolve_evidence(
            ConnectorFamily::Slack,
            "slack-1",
            // The result also carries a canonicalResourceId, which must be ignored.
            &json!({"ts": "1723459200.123", "canonicalResourceId": "attacker-chosen"}),
        )
        .unwrap();
        assert_eq!(resolved.canonical_resource_id, "slack:slack-1:1723459200.123");
        assert_eq!(resolved.source_kind, "slack.message");
    }

    #[test]
    fn the_same_native_id_on_two_accounts_is_two_things() {
        // Namespaced by instance, so one account's message cannot be mistaken for
        // another's just because the provider numbers them the same way.
        let one = resolve_evidence(ConnectorFamily::Gmail, "gmail-work", &json!({"threadId": "t1"})).unwrap();
        let other = resolve_evidence(ConnectorFamily::Gmail, "gmail-personal", &json!({"threadId": "t1"})).unwrap();
        assert_ne!(one.canonical_resource_id, other.canonical_resource_id);
    }

    #[test]
    fn a_result_with_no_recognisable_id_yields_no_evidence() {
        // Evidence without a canonical id cannot be recognised again, so it is not
        // recorded at all rather than recorded with something guessed.
        for family in ConnectorFamily::ALL {
            assert!(resolve_evidence(family, "instance", &json!({"body": "text"})).is_none());
            assert!(resolve_evidence(family, "instance", &json!({})).is_none());
        }
    }

    #[test]
    fn every_family_derives_an_id_from_its_own_shape() {
        let cases = [
            (ConnectorFamily::Slack, json!({"ts": "1.1"}), "slack.message"),
            (ConnectorFamily::Gmail, json!({"threadId": "abc"}), "gmail.thread"),
            (ConnectorFamily::GitHub, json!({"nodeId": "PR_1"}), "github.item"),
            (ConnectorFamily::Linear, json!({"identifier": "BRD-418"}), "linear.issue"),
            (ConnectorFamily::Notion, json!({"pageId": "p1"}), "notion.page"),
        ];
        for (family, result, kind) in cases {
            let resolved = resolve_evidence(family, "i", &result).unwrap();
            assert_eq!(resolved.source_kind, kind);
            assert!(resolved.canonical_resource_id.starts_with(family.as_str()));
        }
    }

    // -----------------------------------------------------------------------
    // Targets
    // -----------------------------------------------------------------------

    #[test]
    fn a_target_outside_the_connector_allowlist_is_refused() {
        for url in [
            "https://evil.example/thread/1",
            "https://github.com.evil.example/x",  // suffix trick
            "https://evil.example/?slack.com",
            "http://slack.com/archives/1",        // not https
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user@evil.example/x",        // userinfo
            "https://slack.com@evil.example/x",   // allowlisted name as userinfo
            "//slack.com/x",
            "",
        ] {
            assert_eq!(
                resolve_target(ConnectorFamily::Slack, &json!({"permalink": url})),
                EvidenceTarget::None,
                "{url:?} must not become a link"
            );
        }
    }

    #[test]
    fn an_allowlisted_permalink_becomes_a_link_with_its_host() {
        let target = resolve_target(
            ConnectorFamily::Slack,
            &json!({"permalink": "https://app.slack.com/archives/C1/p1723459200123"}),
        );
        assert_eq!(
            target,
            EvidenceTarget::ExternalLink {
                url: "https://app.slack.com/archives/C1/p1723459200123".into(),
                host: "app.slack.com".into(),
            }
        );
    }

    #[test]
    fn a_hosts_case_and_port_do_not_smuggle_it_past_the_allowlist() {
        // Case is normalised because hosts are case-insensitive; a port is stripped
        // before matching so the host itself is what is compared.
        assert!(matches!(
            safe_external_link(ConnectorFamily::GitHub, "https://GitHub.com/o/r/pull/1"),
            Some(EvidenceTarget::ExternalLink { .. })
        ));
        assert!(safe_external_link(ConnectorFamily::GitHub, "https://notgithub.com/x").is_none());
    }

    #[test]
    fn one_familys_host_is_not_anothers() {
        assert!(safe_external_link(ConnectorFamily::Slack, "https://github.com/o/r").is_none());
        assert!(safe_external_link(ConnectorFamily::GitHub, "https://slack.com/x").is_none());
    }

    #[test]
    fn a_bridge_local_target_needs_no_allowlist() {
        // A session is not a link out, so nothing to allowlist.
        let target = EvidenceTarget::Session { session_id: "session-1".into() };
        assert!(matches!(target, EvidenceTarget::Session { .. }));
    }

    #[test]
    fn a_result_with_no_permalink_has_no_target_and_that_is_not_an_error() {
        let resolved = resolve_evidence(ConnectorFamily::Notion, "n1", &json!({"pageId": "p1"})).unwrap();
        assert_eq!(resolved.target, EvidenceTarget::None);
    }

    #[test]
    fn every_family_declares_at_least_one_host() {
        // A family with an empty allowlist would silently produce no links at all,
        // which reads as "this connector has no permalinks" rather than as a gap.
        for family in ConnectorFamily::ALL {
            assert!(!family.allowed_hosts().is_empty(), "{family:?} needs hosts");
            assert!(ConnectorFamily::parse(family.as_str()) == Some(family));
        }
    }
}
