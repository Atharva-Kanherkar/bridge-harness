//! Briefing authority: what a model may touch while reading connected tools.
//!
//! This is deliberately not a rung on [`crate::delegation::WriteMode`]. A write
//! mode answers "how much of the filesystem may this agent change", and every
//! rung of it still assumes an agent doing work on a repository with a human
//! nearby. A briefing run is the opposite situation: nobody is watching, and the
//! text coming back from a connector is untrusted and may be asking for a shell.
//!
//! So authority here is deny-by-construction. Nothing is available until Bridge
//! supplies an exact reviewed tool identity, and the built-in families a coding
//! agent would normally want are refused by name as well, so a refusal can say
//! which liability it refused rather than only "not on the list".
//!
//! Every way this can fail to be safe resolves to [`BriefingUnsupported`], which
//! carries a reason. An unknown adapter, an uncertified provider version, a
//! permission representation nothing here recognizes, an ambiguous tool name — all
//! of them fail closed and say so. None of them is a silent `false`.

use bridge_protocol::messages as wire;
use serde::{Deserialize, Serialize};

use crate::delegation::WriteMode;

/// Built-in tool families a briefing run must never reach, matched
/// case-insensitively against the name a provider presents.
///
/// This list does not do the securing — [`BriefingRuntimePolicy::decide`] denies
/// anything not explicitly allowed, so a family missing from here is still
/// refused. It exists so a refusal can name the liability, and so each adapter
/// has something concrete to hand its provider as an explicit deny-list.
pub const DENIED_BUILTIN_FAMILIES: &[(&str, &[&str])] = &[
    ("filesystem", &["read", "write", "edit", "multiedit", "notebookedit", "glob", "ls"]),
    ("search", &["grep", "rg", "ripgrep"]),
    ("shell", &["bash", "shell", "sh", "zsh", "exec", "execute", "run", "killshell", "bashoutput"]),
    ("web", &["webfetch", "websearch", "fetch", "browse", "browser"]),
    ("skill", &["skill", "slashcommand"]),
    ("subagent", &["task", "agent", "spawn", "delegate"]),
    ("computer_use", &["computer", "screenshot", "mouse", "keyboard"]),
];

/// An exact connector tool identity, as reviewed and supplied by Bridge.
///
/// Compared exactly: not by prefix, not case-insensitively, not as a pattern. A
/// reviewed `search` does not admit `search_and_update`, and the difference
/// between the two is the entire point of the review.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BriefingToolIdentity {
    /// The connector instance the tool belongs to.
    pub server: String,
    /// The tool's name as that connector publishes it.
    pub tool: String,
}

impl BriefingToolIdentity {
    /// The MCP wire name providers use for this identity. Bridge stores the
    /// identity; each adapter renders it into its own vocabulary, so the policy
    /// itself stays provider-neutral.
    pub fn wire_name(&self) -> String {
        format!("mcp__{}__{}", self.server, self.tool)
    }
}

/// Why briefing is not available. Every variant carries something a human can
/// read, because "unsupported" with no reason is indistinguishable from a bug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BriefingUnsupported {
    /// An adapter id with no entry in the built-in contract table.
    UnknownAdapter { adapter: String },
    /// A known adapter that cannot enforce this authority.
    AdapterCannotEnforce { adapter: String, reason: String },
    /// The provider is not the version the conformance suite certified.
    UncertifiedProviderVersion {
        adapter: String,
        found: String,
        certified: String,
    },
    /// A permission shape the policy compiler does not understand. Never read as
    /// "no restrictions".
    UnrecognizedPermissionRepresentation { adapter: String, detail: String },
    /// Two reviewed identities render to the same wire name, so neither can be
    /// matched exactly.
    AmbiguousToolNames { name: String },
    /// The tools a provider presents are not the ones the policy was compiled
    /// against.
    ToolListDrift { expected: String, presented: String },
    /// The policy itself does not describe a safe run.
    MalformedPolicy { detail: String },
    /// Briefing asked for a writable tree. Not merged down to read-only — a
    /// caller that asked for both does not agree with itself.
    WritableWorkspaceRequested { write_mode: String },
}

impl BriefingUnsupported {
    /// One line, for an event payload or a log.
    pub fn reason(&self) -> String {
        match self {
            Self::UnknownAdapter { adapter } => {
                format!("`{adapter}` has no built-in briefing contract, so it cannot be trusted with one")
            }
            Self::AdapterCannotEnforce { adapter, reason } => {
                format!("`{adapter}` cannot enforce briefing authority: {reason}")
            }
            Self::UncertifiedProviderVersion { adapter, found, certified } => format!(
                "`{adapter}` reports version `{found}`, but the conformance suite certified `{certified}`"
            ),
            Self::UnrecognizedPermissionRepresentation { adapter, detail } => format!(
                "`{adapter}` described its permissions in a shape Bridge does not recognize: {detail}"
            ),
            Self::AmbiguousToolNames { name } => {
                format!("two reviewed tools both present as `{name}`, so neither can be matched exactly")
            }
            Self::ToolListDrift { expected, presented } => format!(
                "the provider presented a different tool list than the policy was compiled against (expected {expected}, got {presented})"
            ),
            Self::MalformedPolicy { detail } => format!("the briefing policy is not usable: {detail}"),
            Self::WritableWorkspaceRequested { write_mode } => format!(
                "briefing was requested alongside write mode `{write_mode}`, which would give it a writable tree"
            ),
        }
    }
}

/// Why one tool call was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BriefingDenial {
    /// A built-in family briefing never gets, named so the refusal is legible.
    BuiltInFamily { family: String, tool: String },
    /// Not among the reviewed identities. The default answer.
    NotReviewed { tool: String },
    /// Arguments too large to dispatch. Checked before the call goes out, since
    /// the cost of an oversized argument is paid by whoever receives it.
    ArgumentsTooLarge { tool: String, bytes: usize, limit: usize },
}

impl BriefingDenial {
    pub fn reason(&self) -> String {
        match self {
            Self::BuiltInFamily { family, tool } => {
                format!("`{tool}` is a {family} tool, which a briefing run never gets")
            }
            Self::NotReviewed { tool } => {
                format!("`{tool}` is not one of the reviewed connector reads for this run")
            }
            Self::ArgumentsTooLarge { tool, bytes, limit } => {
                format!("the arguments for `{tool}` are {bytes} bytes, over the {limit}-byte limit")
            }
        }
    }
}

/// What may happen to one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDecision {
    Allow,
    Deny(BriefingDenial),
}

impl ToolDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// The authority one briefing run executes under.
///
/// Built by [`Self::compile`], which is the only way to get one: the checks it
/// performs are what make the resulting value safe to hand to an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefingRuntimePolicy {
    /// Reviewed identities, sorted and deduplicated by [`Self::compile`].
    allowed: Vec<BriefingToolIdentity>,
    /// The run's ceilings, from the settings slice 1 contracted.
    limits: wire::WorkBriefLimits,
    /// Largest argument payload one call may carry.
    max_argument_bytes: usize,
    /// The tool list this policy was compiled against, so drift is detectable
    /// rather than something the run discovers by succeeding at the wrong thing.
    compiled_against: Vec<String>,
}

/// Default ceiling on one call's arguments. Generous for a connector query,
/// nowhere near enough to smuggle a payload through.
pub const DEFAULT_MAX_ARGUMENT_BYTES: usize = 8 * 1024;

impl BriefingRuntimePolicy {
    /// Compile reviewed identities into an enforceable policy.
    ///
    /// `presented` is the tool list the provider actually offers. It is recorded
    /// so a later run can detect drift, and checked now so a policy naming a tool
    /// the provider does not have is a refusal rather than a surprise.
    pub fn compile(
        allowed: Vec<BriefingToolIdentity>,
        limits: wire::WorkBriefLimits,
        presented: &[String],
    ) -> Result<Self, BriefingUnsupported> {
        if limits.max_wall_seconds <= 0 || limits.max_turns <= 0 || limits.max_tool_calls <= 0 {
            return Err(BriefingUnsupported::MalformedPolicy {
                detail: format!(
                    "limits must all be positive (wall {}s, turns {}, tool calls {})",
                    limits.max_wall_seconds, limits.max_turns, limits.max_tool_calls
                ),
            });
        }

        let mut sorted = allowed;
        sorted.sort();
        sorted.dedup();

        for identity in &sorted {
            if identity.server.trim().is_empty() || identity.tool.trim().is_empty() {
                return Err(BriefingUnsupported::MalformedPolicy {
                    detail: "a reviewed identity has an empty server or tool name".into(),
                });
            }
            // An identity that renders to a built-in name would let a reviewed
            // entry re-admit something the deny-list exists to keep out.
            if let Some(family) = builtin_family(&identity.tool) {
                return Err(BriefingUnsupported::MalformedPolicy {
                    detail: format!(
                        "`{}` collides with the {family} built-in family and cannot be reviewed in",
                        identity.tool
                    ),
                });
            }
        }

        // Exact matching is only meaningful if the rendered names are unique.
        let mut names: Vec<String> = sorted.iter().map(BriefingToolIdentity::wire_name).collect();
        names.sort();
        if let Some(duplicate) = first_duplicate(&names) {
            return Err(BriefingUnsupported::AmbiguousToolNames { name: duplicate });
        }

        // A reviewed tool the provider does not present cannot be called, and a
        // policy that believes otherwise is one nobody has checked. Not guarded on
        // `presented` being non-empty: reviewing tools in against a provider that
        // offers none is exactly the case that must not compile.
        if let Some(missing) = names.iter().find(|name| !presented.contains(name)) {
            return Err(BriefingUnsupported::ToolListDrift {
                expected: missing.clone(),
                presented: format!("a list of {} that does not contain it", presented.len()),
            });
        }

        let mut compiled_against = presented.to_vec();
        compiled_against.sort();
        Ok(Self {
            allowed: sorted,
            limits,
            max_argument_bytes: DEFAULT_MAX_ARGUMENT_BYTES,
            compiled_against,
        })
    }

    /// Refuse a briefing run that also asked for a writable tree.
    pub fn check_write_mode(write_mode: Option<WriteMode>) -> Result<(), BriefingUnsupported> {
        match write_mode {
            None | Some(WriteMode::ReadOnly) => Ok(()),
            Some(mode) => Err(BriefingUnsupported::WritableWorkspaceRequested {
                write_mode: format!("{mode:?}"),
            }),
        }
    }

    /// Has the provider's tool list changed since this policy was compiled?
    ///
    /// Drift disables briefing; it never resolves toward more authority.
    pub fn check_for_drift(&self, presented: &[String]) -> Result<(), BriefingUnsupported> {
        let mut current = presented.to_vec();
        current.sort();
        if current == self.compiled_against {
            return Ok(());
        }
        Err(BriefingUnsupported::ToolListDrift {
            expected: format!("{} tools", self.compiled_against.len()),
            presented: format!("{} tools", current.len()),
        })
    }

    /// The one question this type exists to answer.
    pub fn decide(&self, tool: &str, argument_bytes: usize) -> ToolDecision {
        // Reviewed first, so the allowlist is the only thing that can say yes.
        if self.allowed.iter().any(|identity| identity.wire_name() == tool) {
            if argument_bytes > self.max_argument_bytes {
                return ToolDecision::Deny(BriefingDenial::ArgumentsTooLarge {
                    tool: tool.to_owned(),
                    bytes: argument_bytes,
                    limit: self.max_argument_bytes,
                });
            }
            return ToolDecision::Allow;
        }
        // Everything else is refused. Naming the family when we recognize it is
        // for the reader; the refusal does not depend on recognizing it.
        match builtin_family(tool) {
            Some(family) => ToolDecision::Deny(BriefingDenial::BuiltInFamily {
                family: family.to_owned(),
                tool: tool.to_owned(),
            }),
            None => ToolDecision::Deny(BriefingDenial::NotReviewed {
                tool: tool.to_owned(),
            }),
        }
    }

    /// The reviewed identities, for an adapter rendering its allowlist.
    pub fn allowed_wire_names(&self) -> Vec<String> {
        self.allowed.iter().map(BriefingToolIdentity::wire_name).collect()
    }

    /// The connector instances any reviewed tool belongs to. An adapter starts
    /// only these and no others.
    pub fn allowed_servers(&self) -> Vec<String> {
        let mut servers: Vec<String> =
            self.allowed.iter().map(|identity| identity.server.clone()).collect();
        servers.sort();
        servers.dedup();
        servers
    }

    /// Built-in names to hand a provider as an explicit deny-list, for providers
    /// that accept one. Belt to `decide`'s braces.
    pub fn denied_builtin_names() -> Vec<&'static str> {
        DENIED_BUILTIN_FAMILIES
            .iter()
            .flat_map(|(_, tools)| tools.iter().copied())
            .collect()
    }

    pub fn limits(&self) -> &wire::WorkBriefLimits {
        &self.limits
    }

    pub fn max_argument_bytes(&self) -> usize {
        self.max_argument_bytes
    }
}

/// Which built-in family a presented tool name belongs to, if any. Matching
/// ignores case and the `mcp__` prefixing providers apply, because a deny-list
/// that can be sidestepped by capitalisation is decoration.
fn builtin_family(tool: &str) -> Option<&'static str> {
    let bare = tool.rsplit("__").next().unwrap_or(tool).trim().to_lowercase();
    DENIED_BUILTIN_FAMILIES
        .iter()
        .find(|(_, tools)| tools.contains(&bare.as_str()))
        .map(|(family, _)| *family)
}

fn first_duplicate(sorted: &[String]) -> Option<String> {
    sorted
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0].clone())
}

// ---------------------------------------------------------------------------
// Which adapters may be trusted with this authority
// ---------------------------------------------------------------------------

/// How an adapter expresses tool permissions to its provider.
///
/// This is the thing that decides whether briefing is even possible: the policy
/// needs to name one exact connector tool and refuse every other, and a
/// representation that cannot express that cannot enforce it. An unfamiliar shape
/// is [`Self::Unrecognized`] and fails closed — never read as "no restrictions".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PermissionRepresentation {
    /// Claude Agent SDK: a per-call `canUseTool` gate, plus explicit allow and
    /// deny tool lists and an explicit MCP server map. Exact identities are
    /// expressible, and the per-call gate makes them enforceable rather than
    /// merely declared.
    ClaudeAgentSdk,
    /// OpenCode's rule list — `permission` × `pattern` × allow/deny/ask over
    /// coarse families like `edit` and `bash`. There is no vocabulary for one
    /// connector tool's exact identity.
    OpenCodeRuleList,
    /// Codex's app-server sandbox policy — writable roots and network access.
    /// Filesystem authority, with nothing to say about which tools exist.
    CodexSandboxPolicy,
    /// Something Bridge has not been taught to compile a policy into.
    Unrecognized,
}

impl PermissionRepresentation {
    /// Can one exact tool identity be both named and enforced?
    fn can_enforce_exact_identities(self) -> bool {
        matches!(self, Self::ClaudeAgentSdk)
    }

    fn why_not(self) -> &'static str {
        match self {
            Self::ClaudeAgentSdk => "",
            Self::OpenCodeRuleList => {
                "its permission rules cover coarse families like edit and bash, with no vocabulary for one connector tool's exact identity"
            }
            Self::CodexSandboxPolicy => {
                "its sandbox policy governs writable roots and network access, not which tools exist"
            }
            Self::Unrecognized => "Bridge does not recognize how it represents tool permissions",
        }
    }
}

/// Whether an adapter may claim briefing support, and what certified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum BriefingSupport {
    /// The shared conformance suite passed against this provider version. The
    /// version is part of the certification: a different one is uncertified.
    Supported { certified_provider_version: &'static str },
    /// Not available, and why.
    Unsupported { reason: &'static str },
}

/// One adapter's briefing standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefingCapability {
    pub adapter: &'static str,
    pub permissions: PermissionRepresentation,
    pub support: BriefingSupport,
}

/// The version of the Claude Agent SDK the conformance suite runs against. Kept
/// beside the sidecar's dependency range on purpose: a provider that reports
/// something else has not been certified, whatever else is true of it.
pub const CLAUDE_CERTIFIED_SDK_VERSION: &str = "0.3";

/// Every adapter's standing, stated explicitly.
///
/// Deliberately not a field on `builtin_compatibility::BuiltInAgentContract`:
/// four earlier contracts promise that report stays byte-identical at schema v1,
/// and briefing standing is a different kind of fact anyway — it is certified by
/// a conformance suite rather than being install-independent transport metadata.
const BRIEFING_CAPABILITIES: &[BriefingCapability] = &[
    BriefingCapability {
        adapter: "claude",
        permissions: PermissionRepresentation::ClaudeAgentSdk,
        support: BriefingSupport::Supported {
            certified_provider_version: CLAUDE_CERTIFIED_SDK_VERSION,
        },
    },
    BriefingCapability {
        adapter: "codex",
        permissions: PermissionRepresentation::CodexSandboxPolicy,
        support: BriefingSupport::Unsupported {
            reason: "the app-server protocol has no per-tool authority, so an exact connector read cannot be isolated from a mutation",
        },
    },
    BriefingCapability {
        adapter: "opencode",
        permissions: PermissionRepresentation::OpenCodeRuleList,
        support: BriefingSupport::Unsupported {
            reason: "its permission rules are coarse families, so one reviewed connector tool cannot be admitted without admitting its neighbours",
        },
    },
];

pub fn briefing_capabilities() -> &'static [BriefingCapability] {
    BRIEFING_CAPABILITIES
}

/// One adapter's standing, or `None` for an adapter nobody has certified.
pub fn briefing_capability(adapter: &str) -> Option<&'static BriefingCapability> {
    BRIEFING_CAPABILITIES
        .iter()
        .find(|capability| capability.adapter == adapter)
}

/// May this adapter, at this reported version, run a briefing?
///
/// Every path out of here that is not `Ok` names what was wrong. An adapter that
/// says nothing about itself is unsupported: absence is not consent.
/// The boundary check every adapter performs before accepting a briefing policy:
/// may this adapter hold this authority at all?
///
/// Deliberately version-free, so an adapter can refuse at its own boundary
/// without knowing what version anything reports. [`certify_briefing`] adds the
/// version question for the caller configuring a run.
pub fn adapter_may_brief(adapter: &str) -> Result<&'static BriefingCapability, BriefingUnsupported> {
    let Some(capability) = briefing_capability(adapter) else {
        return Err(BriefingUnsupported::UnknownAdapter {
            adapter: adapter.to_owned(),
        });
    };

    // Checked before the suite verdict, because a representation Bridge cannot
    // compile into is a fact about the adapter regardless of what a table claims.
    if !capability.permissions.can_enforce_exact_identities() {
        return match capability.permissions {
            PermissionRepresentation::Unrecognized => {
                Err(BriefingUnsupported::UnrecognizedPermissionRepresentation {
                    adapter: adapter.to_owned(),
                    detail: capability.permissions.why_not().to_owned(),
                })
            }
            _ => Err(BriefingUnsupported::AdapterCannotEnforce {
                adapter: adapter.to_owned(),
                reason: match capability.support {
                    BriefingSupport::Unsupported { reason } => reason.to_owned(),
                    // A table claiming support for something unenforceable is a
                    // bug, and the safe reading of a bug is refusal.
                    BriefingSupport::Supported { .. } => capability.permissions.why_not().to_owned(),
                },
            }),
        };
    }

    match capability.support {
        BriefingSupport::Supported { .. } => Ok(capability),
        BriefingSupport::Unsupported { reason } => Err(BriefingUnsupported::AdapterCannotEnforce {
            adapter: adapter.to_owned(),
            reason: reason.to_owned(),
        }),
    }
}

/// May this adapter, at this reported version, run a briefing?
///
/// Every path out of here that is not `Ok` names what was wrong. An adapter that
/// says nothing about itself is unsupported: absence is not consent.
pub fn certify_briefing(
    adapter: &str,
    reported_provider_version: Option<&str>,
) -> Result<&'static BriefingCapability, BriefingUnsupported> {
    let capability = adapter_may_brief(adapter)?;
    let BriefingSupport::Supported {
        certified_provider_version,
    } = capability.support
    else {
        unreachable!("adapter_may_brief refuses every unsupported adapter")
    };

    // A version nobody ran the suite against is not certified, including no
    // version at all: an adapter that cannot say what it is has not been checked.
    let Some(reported) = reported_provider_version.map(str::trim).filter(|value| !value.is_empty())
    else {
        return Err(BriefingUnsupported::UncertifiedProviderVersion {
            adapter: adapter.to_owned(),
            found: "unreported".to_owned(),
            certified: certified_provider_version.to_owned(),
        });
    };
    if !version_line_matches(reported, certified_provider_version) {
        return Err(BriefingUnsupported::UncertifiedProviderVersion {
            adapter: adapter.to_owned(),
            found: reported.to_owned(),
            certified: certified_provider_version.to_owned(),
        });
    }
    Ok(capability)
}

/// Does a reported version sit on the certified line?
///
/// The certified value is a prefix of dot-separated components — `0.3` certifies
/// `0.3.209` but not `0.30.1`. Compared component-wise rather than as a string,
/// because `"0.3"` is a textual prefix of `"0.30.1"` and those are different
/// releases.
fn version_line_matches(reported: &str, certified: &str) -> bool {
    let mut reported = reported.split('.');
    certified
        .split('.')
        .all(|component| reported.next() == Some(component))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> wire::WorkBriefLimits {
        wire::WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 12,
            max_tool_calls: 24,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        }
    }

    fn identity(server: &str, tool: &str) -> BriefingToolIdentity {
        BriefingToolIdentity {
            server: server.into(),
            tool: tool.into(),
        }
    }

    /// One reviewed read, and the provider offering exactly it.
    fn policy() -> BriefingRuntimePolicy {
        let reviewed = identity("notion", "search");
        let presented = vec![reviewed.wire_name()];
        BriefingRuntimePolicy::compile(vec![reviewed], limits(), &presented).unwrap()
    }

    // -----------------------------------------------------------------------
    // Briefing authority is its own axis
    // -----------------------------------------------------------------------

    #[test]
    fn briefing_authority_is_not_a_write_mode() {
        // Nothing here consumes a WriteMode to decide a tool call, which is the
        // structural half of "separate axis". The other half is that a read-only
        // run is the only write mode briefing tolerates.
        for mode in [WriteMode::ReadOnly] {
            assert!(BriefingRuntimePolicy::check_write_mode(Some(mode)).is_ok());
        }
        assert!(BriefingRuntimePolicy::check_write_mode(None).is_ok());
    }

    #[test]
    fn a_briefing_policy_refuses_a_writable_write_mode() {
        for mode in [WriteMode::Shared, WriteMode::Isolated, WriteMode::Full] {
            let error = BriefingRuntimePolicy::check_write_mode(Some(mode)).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::WritableWorkspaceRequested { .. }),
                "{mode:?} must be refused, not merged down to read-only: {error:?}"
            );
            assert!(error.reason().contains("writable tree"));
        }
    }

    // -----------------------------------------------------------------------
    // Deny by construction
    // -----------------------------------------------------------------------

    #[test]
    fn an_empty_allowlist_denies_every_connector_tool() {
        let policy = BriefingRuntimePolicy::compile(vec![], limits(), &[]).unwrap();
        for tool in ["mcp__notion__search", "mcp__github__list_issues", "anything"] {
            assert_eq!(
                policy.decide(tool, 10),
                ToolDecision::Deny(BriefingDenial::NotReviewed { tool: tool.into() }),
                "nothing is available until Bridge reviews it in"
            );
        }
    }

    #[test]
    fn one_exact_allowlisted_identity_is_permitted() {
        assert_eq!(policy().decide("mcp__notion__search", 128), ToolDecision::Allow);
    }

    #[test]
    fn every_builtin_tool_family_is_denied() {
        let policy = policy();
        for (family, tools) in DENIED_BUILTIN_FAMILIES {
            for tool in *tools {
                let decision = policy.decide(tool, 10);
                assert_eq!(
                    decision,
                    ToolDecision::Deny(BriefingDenial::BuiltInFamily {
                        family: (*family).into(),
                        tool: (*tool).into(),
                    }),
                    "{tool} is a {family} tool and must be refused"
                );
            }
        }
    }

    #[test]
    fn a_builtin_family_is_denied_however_it_is_capitalised_or_prefixed() {
        // A deny-list that a provider's naming convention can slip past is
        // decoration. These are the same tool wearing different hats.
        let policy = policy();
        for tool in ["Bash", "BASH", "mcp__anything__bash", "  bash  ", "WebFetch"] {
            let decision = policy.decide(tool, 10);
            assert!(
                matches!(decision, ToolDecision::Deny(BriefingDenial::BuiltInFamily { .. })),
                "{tool} must be recognized as a built-in family: {decision:?}"
            );
        }
    }

    #[test]
    fn a_read_named_mutation_is_denied() {
        // The misleading-name fixture. A reviewed `search` says nothing about a
        // tool that merely starts with the same letters.
        let policy = policy();
        for tool in [
            "mcp__notion__search_and_update",
            "mcp__notion__search_replace",
            "mcp__notion__update_search_index",
        ] {
            assert_eq!(
                policy.decide(tool, 64),
                ToolDecision::Deny(BriefingDenial::NotReviewed { tool: tool.into() }),
                "{tool} reads like the reviewed tool but is not it"
            );
        }
    }

    #[test]
    fn identity_matching_is_exact() {
        let policy = policy();
        for tool in [
            "mcp__notion__Search",       // case
            "MCP__notion__search",       // case in the prefix
            "mcp__notion__search ",      // trailing space
            " mcp__notion__search",      // leading space
            "mcp__notion__sear",         // prefix of the tool
            "mcp__notionn__search",      // near-miss server
            "mcp__notion__search__v2",   // suffixed
            "notion__search",            // unprefixed
        ] {
            assert!(
                !policy.decide(tool, 64).is_allowed(),
                "{tool:?} is not the reviewed identity and must not be admitted"
            );
        }
    }

    #[test]
    fn oversized_arguments_are_refused_before_dispatch() {
        let policy = policy();
        let limit = policy.max_argument_bytes();
        assert_eq!(policy.decide("mcp__notion__search", limit), ToolDecision::Allow);
        assert_eq!(
            policy.decide("mcp__notion__search", limit + 1),
            ToolDecision::Deny(BriefingDenial::ArgumentsTooLarge {
                tool: "mcp__notion__search".into(),
                bytes: limit + 1,
                limit,
            }),
            "the size check belongs before the call goes out, not after"
        );
    }

    // -----------------------------------------------------------------------
    // Compilation fails closed
    // -----------------------------------------------------------------------

    #[test]
    fn duplicate_normalized_tool_names_disable_briefing() {
        // Two connector instances whose names collide once rendered. Neither can
        // be matched exactly, so neither is usable.
        let error = BriefingRuntimePolicy::compile(
            vec![identity("a__b", "c"), identity("a", "b__c")],
            limits(),
            &[],
        )
        .unwrap_err();
        assert!(
            matches!(error, BriefingUnsupported::AmbiguousToolNames { .. }),
            "{error:?}"
        );
        assert!(error.reason().contains("matched exactly"));
    }

    #[test]
    fn the_same_identity_twice_is_not_ambiguous() {
        // Deduplication, not a refusal: asking for one thing twice is one thing.
        let reviewed = identity("notion", "search");
        let policy = BriefingRuntimePolicy::compile(
            vec![reviewed.clone(), reviewed.clone()],
            limits(),
            &[reviewed.wire_name()],
        )
        .unwrap();
        assert_eq!(policy.allowed_wire_names(), vec!["mcp__notion__search"]);
    }

    #[test]
    fn a_reviewed_tool_the_provider_does_not_offer_is_drift() {
        let error = BriefingRuntimePolicy::compile(
            vec![identity("notion", "search")],
            limits(),
            &["mcp__github__list_issues".to_owned()],
        )
        .unwrap_err();
        assert!(matches!(error, BriefingUnsupported::ToolListDrift { .. }), "{error:?}");
    }

    #[test]
    fn a_tool_list_that_drifted_disables_briefing() {
        let policy = policy();
        assert!(policy.check_for_drift(&["mcp__notion__search".to_owned()]).is_ok());
        // A tool appearing is drift too: the policy was reviewed against a list,
        // and a longer list is not the list that was reviewed.
        let error = policy
            .check_for_drift(&[
                "mcp__notion__search".to_owned(),
                "mcp__notion__update".to_owned(),
            ])
            .unwrap_err();
        assert!(matches!(error, BriefingUnsupported::ToolListDrift { .. }), "{error:?}");
        assert!(policy.check_for_drift(&[]).is_err(), "an empty list is drift as well");
    }

    #[test]
    fn a_reviewed_identity_cannot_smuggle_in_a_builtin_family() {
        let error = BriefingRuntimePolicy::compile(vec![identity("evil", "bash")], limits(), &[])
            .unwrap_err();
        assert!(matches!(error, BriefingUnsupported::MalformedPolicy { .. }), "{error:?}");
        assert!(error.reason().contains("shell"));
    }

    #[test]
    fn a_malformed_policy_fails_closed() {
        // Non-positive limits describe a run that either never stops or never
        // starts. Either way it is not something to execute.
        for (wall, turns, calls) in [(0, 12, 24), (600, 0, 24), (600, 12, 0), (-1, 12, 24)] {
            let broken = wire::WorkBriefLimits {
                max_wall_seconds: wall,
                max_turns: turns,
                max_tool_calls: calls,
                max_output_tokens: None,
                cost_ceiling_microusd: None,
            };
            let error = BriefingRuntimePolicy::compile(vec![], broken, &[]).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::MalformedPolicy { .. }),
                "({wall}, {turns}, {calls}) must not compile: {error:?}"
            );
        }
        let empty_name = BriefingRuntimePolicy::compile(vec![identity("", "search")], limits(), &[])
            .unwrap_err();
        assert!(matches!(empty_name, BriefingUnsupported::MalformedPolicy { .. }));
    }

    // -----------------------------------------------------------------------
    // Which adapters may be trusted with this authority
    // -----------------------------------------------------------------------

    #[test]
    fn every_adapter_defaults_to_briefing_unsupported() {
        // Absence is not consent. An adapter nobody certified gets no authority,
        // and the refusal names it rather than being a bare false.
        for adapter in ["", "gemini", "aider", "some-future-harness", "CLAUDE"] {
            let error = certify_briefing(adapter, Some("0.3.209")).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::UnknownAdapter { .. }),
                "{adapter:?} must be unsupported: {error:?}"
            );
            assert!(error.reason().contains("no built-in briefing contract"));
        }
    }

    #[test]
    fn an_unknown_adapter_id_is_unsupported_with_a_reason() {
        let error = certify_briefing("gemini", None).unwrap_err();
        assert_eq!(
            error,
            BriefingUnsupported::UnknownAdapter { adapter: "gemini".into() }
        );
        assert!(error.reason().contains("gemini"));
    }

    #[test]
    fn claude_declares_briefing_support() {
        let capability = certify_briefing("claude", Some("0.3.209")).unwrap();
        assert_eq!(capability.permissions, PermissionRepresentation::ClaudeAgentSdk);
        assert!(matches!(capability.support, BriefingSupport::Supported { .. }));
    }

    #[test]
    fn codex_declares_no_briefing_support_with_a_reason() {
        let error = certify_briefing("codex", Some("1.0.0")).unwrap_err();
        let BriefingUnsupported::AdapterCannotEnforce { adapter, reason } = &error else {
            panic!("expected a cannot-enforce refusal, got {error:?}");
        };
        assert_eq!(adapter, "codex");
        assert!(reason.contains("per-tool authority"), "{reason}");
    }

    #[test]
    fn opencode_declares_no_briefing_support_with_a_reason() {
        let error = certify_briefing("opencode", Some("1.0.0")).unwrap_err();
        let BriefingUnsupported::AdapterCannotEnforce { adapter, reason } = &error else {
            panic!("expected a cannot-enforce refusal, got {error:?}");
        };
        assert_eq!(adapter, "opencode");
        assert!(reason.contains("coarse families"), "{reason}");
    }

    #[test]
    fn an_unsupported_adapter_never_falls_back_to_another() {
        // The refusal names the adapter that was asked for. Nothing in this path
        // can answer "use Claude instead" — a silent substitution would run a
        // briefing on a provider the caller did not choose.
        for adapter in ["codex", "opencode"] {
            let error = certify_briefing(adapter, Some("1.0.0")).unwrap_err();
            let reason = error.reason();
            assert!(reason.contains(adapter), "{reason}");
            assert!(!reason.contains("claude"), "no fallback may be suggested: {reason}");
        }
    }

    #[test]
    fn a_provider_version_the_suite_did_not_certify_is_unsupported() {
        for version in ["0.2.999", "0.4.0", "1.0.0", "0.30.1"] {
            let error = certify_briefing("claude", Some(version)).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::UncertifiedProviderVersion { .. }),
                "{version} must not be certified: {error:?}"
            );
            assert!(error.reason().contains(version));
        }
        // "0.3" is a textual prefix of "0.30.1", and those are different releases.
        assert!(certify_briefing("claude", Some("0.3.0")).is_ok());
        assert!(certify_briefing("claude", Some("0.3")).is_ok());
    }

    #[test]
    fn a_provider_that_cannot_say_what_it_is_has_not_been_certified() {
        for reported in [None, Some(""), Some("   ")] {
            let error = certify_briefing("claude", reported).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::UncertifiedProviderVersion { .. }),
                "{reported:?}: {error:?}"
            );
        }
    }

    #[test]
    fn an_unrecognized_permission_representation_is_unsupported_not_empty() {
        // The shape Bridge has not been taught to compile into. The danger is
        // reading an unfamiliar representation as "nothing is restricted", so this
        // pins the opposite.
        assert!(!PermissionRepresentation::Unrecognized.can_enforce_exact_identities());
        for representation in [
            PermissionRepresentation::OpenCodeRuleList,
            PermissionRepresentation::CodexSandboxPolicy,
            PermissionRepresentation::Unrecognized,
        ] {
            assert!(
                !representation.can_enforce_exact_identities(),
                "{representation:?} cannot name one exact tool and refuse the rest"
            );
            assert!(!representation.why_not().is_empty());
        }
        assert!(PermissionRepresentation::ClaudeAgentSdk.can_enforce_exact_identities());
    }

    #[test]
    fn every_registered_adapter_has_an_explicit_briefing_verdict() {
        // The acceptance criterion: all three are stated, none is silent.
        let adapters: Vec<&str> = briefing_capabilities()
            .iter()
            .map(|capability| capability.adapter)
            .collect();
        assert_eq!(adapters, vec!["claude", "codex", "opencode"]);
        for capability in briefing_capabilities() {
            match capability.support {
                BriefingSupport::Supported {
                    certified_provider_version,
                } => assert!(!certified_provider_version.is_empty()),
                BriefingSupport::Unsupported { reason } => assert!(
                    reason.len() > 20,
                    "{} needs a reason worth reading, got {reason:?}",
                    capability.adapter
                ),
            }
        }
    }

    #[test]
    fn only_a_representation_with_a_per_call_gate_may_claim_support() {
        // A table entry claiming support for something unenforceable is a bug, and
        // the safe reading of a bug is refusal, not the claim.
        for capability in briefing_capabilities() {
            if matches!(capability.support, BriefingSupport::Supported { .. }) {
                assert!(
                    capability.permissions.can_enforce_exact_identities(),
                    "{} claims support without a per-call gate",
                    capability.adapter
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // The adapter boundary
    // -----------------------------------------------------------------------

    #[test]
    fn a_briefing_start_on_an_unsupported_adapter_is_refused_at_the_boundary() {
        // The boundary check is version-free on purpose: an adapter refuses on its
        // own account, without needing to know what anything reports.
        for adapter in ["codex", "opencode"] {
            let error = adapter_may_brief(adapter).unwrap_err();
            assert!(
                matches!(error, BriefingUnsupported::AdapterCannotEnforce { .. }),
                "{adapter}: {error:?}"
            );
        }
        assert!(adapter_may_brief("claude").is_ok());
        assert!(matches!(
            adapter_may_brief("gemini").unwrap_err(),
            BriefingUnsupported::UnknownAdapter { .. }
        ));
    }

    #[test]
    fn the_boundary_check_and_the_version_check_agree_on_who_may_brief() {
        // Two entry points, one answer about the adapter itself. If these ever
        // disagree, one of them is a way in.
        for adapter in ["claude", "codex", "opencode", "gemini"] {
            let boundary = adapter_may_brief(adapter).is_ok();
            let certified = certify_briefing(adapter, Some("0.3.209")).is_ok();
            assert_eq!(
                boundary, certified,
                "{adapter}: the boundary and the certified answer must not diverge"
            );
        }
    }

    // -----------------------------------------------------------------------
    // What an adapter is handed
    // -----------------------------------------------------------------------

    #[test]
    fn reviewing_tools_in_against_a_provider_that_offers_none_does_not_compile() {
        // Found in review: this used to be skipped when the presented list was
        // empty, which let a policy claim authority nobody had checked against a
        // real provider. Run time would have caught it as drift, but a policy that
        // cannot possibly work should not be constructible.
        let error =
            BriefingRuntimePolicy::compile(vec![identity("notion", "search")], limits(), &[])
                .unwrap_err();
        assert!(matches!(error, BriefingUnsupported::ToolListDrift { .. }), "{error:?}");
    }

    #[test]
    fn an_adapter_is_told_only_the_servers_it_needs() {
        let policy = BriefingRuntimePolicy::compile(
            vec![
                identity("notion", "search"),
                identity("notion", "fetch_page"),
                identity("github", "list_issues"),
            ],
            limits(),
            &[
                "mcp__notion__search".to_owned(),
                "mcp__notion__fetch_page".to_owned(),
                "mcp__github__list_issues".to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(policy.allowed_servers(), vec!["github", "notion"]);
        assert_eq!(
            policy.allowed_wire_names(),
            vec![
                "mcp__github__list_issues",
                "mcp__notion__fetch_page",
                "mcp__notion__search"
            ],
            "sorted, so what an adapter is handed does not depend on review order"
        );
    }

    #[test]
    fn the_builtin_denylist_covers_every_family() {
        let names = BriefingRuntimePolicy::denied_builtin_names();
        for (_, tools) in DENIED_BUILTIN_FAMILIES {
            for tool in *tools {
                assert!(names.contains(tool), "{tool} must be in the explicit deny-list");
            }
        }
        assert!(names.contains(&"bash") && names.contains(&"webfetch") && names.contains(&"task"));
    }
}
