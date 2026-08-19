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
