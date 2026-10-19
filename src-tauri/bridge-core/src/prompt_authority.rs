//! Provider prompt authority: what Bridge may do to the PROVIDER BASE prompt
//! layer — the system-level baseline each provider ships before Bridge adds a
//! single word of its own (Claude's `claude_code` preset, Codex's own agent
//! instructions, OpenCode's provider default).
//!
//! This is a narrower question than [`crate::briefing_policy`]'s tool
//! authority: three static verdicts per adapter rather than a runtime decision
//! loop. Can Bridge *read* that base text back? Can it *append* to it without
//! disturbing it? Can it *replace* it outright?
//!
//! Same discipline as briefing_policy: deny-by-construction. A verdict is
//! `Supported` only once Bridge has read how the adapter actually delivers
//! prompt text today and can point at the code that proves it — not because
//! the capability would be convenient or because a provider's docs claim it
//! in general. An adapter nobody has registered gets
//! [`PromptAuthorityUnsupported::UnknownAdapter`], never a silent `false`, and
//! a capability that would need proof nobody has collected yet stays
//! `Unsupported` with a reason naming exactly what is missing.
//!
//! This module states verdicts. It does not wire any of them anywhere: no UI,
//! no `StartRequest` field, no aggregate token report reads this yet. Wiring a
//! capability from an assumption or from a provider's aggregate token report
//! is the one thing this slice must not do — see the issue this module was
//! written for (Prompt Studio 2/8: provider prompt authority).

use serde::{Deserialize, Serialize};

/// Why a provider-base prompt-authority verdict could not be produced for an
/// adapter — currently only because nobody registered one.
///
/// Kept as its own type rather than folding "unknown adapter" into
/// [`PromptVerdict`], because an unregistered adapter has no capability row to
/// answer from at all. That is a different failure than a registered adapter
/// answering `false` for one specific capability, and the two must not read
/// the same to a caller deciding whether it is safe to proceed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PromptAuthorityUnsupported {
    /// An adapter id with no entry in the built-in authority table.
    UnknownAdapter { adapter: String },
}

impl PromptAuthorityUnsupported {
    /// One line, for an event payload or a log.
    pub fn reason(&self) -> String {
        match self {
            Self::UnknownAdapter { adapter } => format!(
                "`{adapter}` has no registered provider-base prompt authority, so it cannot be trusted with one"
            ),
        }
    }
}

/// One capability's standing for the provider-base prompt layer.
///
/// `Unsupported` always carries a reason, exactly like
/// [`crate::briefing_policy::BriefingUnsupported`] — "unsupported" with no
/// reason is indistinguishable from a bug. `Supported` carries nothing extra:
/// unlike briefing's per-tool authority, there is no compiled policy object
/// downstream that needs a certified version pinned to it. The evidence for a
/// `Supported` verdict is the adapter source cited in the doc comment beside
/// it in [`PROVIDER_BASE_PROMPT_AUTHORITY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum PromptVerdict {
    Supported,
    Unsupported { reason: &'static str },
}

impl PromptVerdict {
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// One adapter's standing against the PROVIDER BASE prompt layer.
///
/// `readable`: can Bridge read back the text the provider actually sent as
/// its base prompt? `appendable`: can Bridge add to that base without
/// disturbing it? `replaceable`: can Bridge supply the entire base prompt
/// itself, in place of the provider's own? None of the three implies another
/// — an adapter can be `appendable` and not `readable`, which is exactly
/// Claude's standing below: Bridge has appended to the `claude_code` preset
/// since before this module existed, but the SDK never hands back the
/// preset's own compiled text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBasePromptAuthority {
    pub adapter: &'static str,
    pub readable: PromptVerdict,
    pub appendable: PromptVerdict,
    pub replaceable: PromptVerdict,
}

/// Every adapter's standing, stated explicitly, against how each one actually
/// delivers prompt text today:
///
/// - **Claude** (`claude_adapter.rs` / `sidecar/claude-agent/options.mjs`):
///   `buildOptions` always sends
///   `systemPrompt: { type: "preset", preset: "claude_code", append: instructions }`.
///   That is Bridge appending to a named preset, and it is the only capability
///   in this table with shipped code as its proof. The SDK compiles the
///   preset internally and never returns the compiled text, and nobody has
///   proven what a bare string does to it — see the `replaceable` reason.
/// - **Codex** (`codex_adapter.rs`): both `thread_start_params` and
///   `thread_resume_params` send `developerInstructions`, the one channel the
///   app-server reads. Appending is proven by a live cache experiment
///   (docs/managed-agent-runtimes.md, "Codex prompt prefix"): the cached
///   prefix shared across threads is the same ~13k tokens whatever the
///   developer text is, so that text sits after Codex's base prompt, and the
///   model follows it. Reading and replacing remain unproven.
/// - **OpenCode** (`opencode_adapter.rs`): `send_turn_with_context` rebuilds
///   the `system` field from `instructions` from scratch on *every* turn. The
///   bytes are now constant for the launch — per-turn context rides in the
///   prompt parts, so a turn carrying a credential contract no longer mutates
///   `system` — but it is still a fresh value handed to the provider each
///   time, not a base Bridge has ever read back or observed being layered
///   onto.
///
/// Not a field on any other contract table: this is its own kind of fact,
/// certified by reading each adapter's delivery code rather than by a
/// conformance suite.
const PROVIDER_BASE_PROMPT_AUTHORITY: &[ProviderBasePromptAuthority] = &[
    ProviderBasePromptAuthority {
        adapter: "claude",
        readable: PromptVerdict::Unsupported {
            reason: "the Claude Agent SDK compiles the `claude_code` preset internally and exposes no accessor for the resulting base prompt text, so Bridge cannot read what it actually sent",
        },
        // Matches options.mjs today: `{ type: "preset", preset: "claude_code", append: instructions }`.
        appendable: PromptVerdict::Supported,
        replaceable: PromptVerdict::Unsupported {
            reason: "bare-string replacement semantics have not been proven against the installed Claude Agent SDK — see sidecar/claude-agent/test/system-prompt-replacement.integration.mjs, which requires live authenticated credentials and has not been run against this checkout; until it passes, options.mjs keeps sending the preset+append form unchanged",
        },
    },
    ProviderBasePromptAuthority {
        adapter: "codex",
        readable: PromptVerdict::Unsupported {
            reason: "the app-server protocol has no method that returns Codex's own base agent instructions, so Bridge cannot read what it actually is",
        },
        // Measured against codex-cli 0.159.3, see the doc comment above.
        appendable: PromptVerdict::Supported,
        replaceable: PromptVerdict::Unsupported {
            reason: "developerInstructions layers after Codex's base prompt rather than superseding it; replacing the base would need baseInstructions on thread/start, which Bridge does not send and has not verified",
        },
    },
    ProviderBasePromptAuthority {
        adapter: "opencode",
        readable: PromptVerdict::Unsupported {
            reason: "OpenCode's session API has no endpoint that returns its provider's base system prompt",
        },
        appendable: PromptVerdict::Unsupported {
            reason: "send_turn_with_context reconstructs the `system` field from scratch on every turn instead of layering onto a base Bridge has observed, so whether OpenCode appends this to its own default or overwrites it is unverified — holding those bytes constant across turns does not make the layering observable",
        },
        replaceable: PromptVerdict::Unsupported {
            reason: "the same reconstruction on every turn makes replace unprovable: OpenCode's layering behavior against its provider's own base prompt is undocumented and unverified",
        },
    },
];

pub fn provider_base_prompt_authorities() -> &'static [ProviderBasePromptAuthority] {
    PROVIDER_BASE_PROMPT_AUTHORITY
}

/// One adapter's standing, or an explained refusal for an adapter nobody has
/// registered. Absence is not consent: an adapter that says nothing about
/// itself gets no authority.
pub fn provider_base_prompt_authority(
    adapter: &str,
) -> Result<&'static ProviderBasePromptAuthority, PromptAuthorityUnsupported> {
    PROVIDER_BASE_PROMPT_AUTHORITY
        .iter()
        .find(|capability| capability.adapter == adapter)
        .ok_or_else(|| PromptAuthorityUnsupported::UnknownAdapter {
            adapter: adapter.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // The acceptance criterion: all three are stated, none is silent.
    // -----------------------------------------------------------------------

    #[test]
    fn every_registered_adapter_has_an_explicit_prompt_authority_verdict() {
        let adapters: Vec<&str> =
            provider_base_prompt_authorities().iter().map(|capability| capability.adapter).collect();
        assert_eq!(adapters, vec!["claude", "codex", "opencode"]);
        for capability in provider_base_prompt_authorities() {
            for (name, verdict) in [
                ("readable", capability.readable),
                ("appendable", capability.appendable),
                ("replaceable", capability.replaceable),
            ] {
                if let PromptVerdict::Unsupported { reason } = verdict {
                    assert!(
                        reason.len() > 20,
                        "{}.{name} needs a reason worth reading, got {reason:?}",
                        capability.adapter
                    );
                }
            }
        }
    }

    #[test]
    fn an_unregistered_adapter_is_unsupported_with_a_reason_not_a_silent_false() {
        for adapter in ["", "gemini", "aider", "some-future-harness", "CLAUDE"] {
            let error = provider_base_prompt_authority(adapter).unwrap_err();
            assert!(
                matches!(error, PromptAuthorityUnsupported::UnknownAdapter { .. }),
                "{adapter:?} must be unsupported: {error:?}"
            );
            assert!(error.reason().contains("no registered provider-base prompt authority"));
        }
    }

    // -----------------------------------------------------------------------
    // Claude: append is shipped and provable; replace and read are not.
    // -----------------------------------------------------------------------

    #[test]
    fn claude_may_append_via_its_shipped_preset() {
        let claude = provider_base_prompt_authority("claude").unwrap();
        assert!(
            claude.appendable.is_supported(),
            "options.mjs already ships append via the claude_code preset"
        );
    }

    #[test]
    fn claude_replace_stays_refused_until_the_sdk_test_proves_it() {
        let claude = provider_base_prompt_authority("claude").unwrap();
        assert!(
            !claude.replaceable.is_supported(),
            "replace must stay refused until the bare-string SDK integration test proves it"
        );
        let reason = claude.replaceable.reason().unwrap();
        assert!(reason.contains("Agent SDK"), "{reason}");
        assert!(
            reason.contains("system-prompt-replacement.integration"),
            "the refusal must point at the test that would resolve it: {reason}"
        );
    }

    #[test]
    fn claude_cannot_read_back_the_compiled_preset() {
        let claude = provider_base_prompt_authority("claude").unwrap();
        assert!(!claude.readable.is_supported());
    }

    // -----------------------------------------------------------------------
    // Codex and OpenCode: nothing claimed beyond what their adapters prove.
    // -----------------------------------------------------------------------

    #[test]
    fn codex_and_opencode_claim_nothing_beyond_what_their_adapters_prove() {
        for adapter in ["codex", "opencode"] {
            let capability = provider_base_prompt_authority(adapter).unwrap();
            let mut verdicts = vec![
                ("readable", capability.readable),
                ("replaceable", capability.replaceable),
            ];
            // Codex append is the one proven claim: the live prefix-cache experiment.
            if adapter != "codex" {
                verdicts.push(("appendable", capability.appendable));
            }
            for (name, verdict) in verdicts {
                assert!(
                    !verdict.is_supported(),
                    "{adapter}.{name} must not be claimed without proof from that adapter's own delivery code"
                );
            }
        }
    }

    #[test]
    fn codex_may_append_via_developer_instructions() {
        let codex = provider_base_prompt_authority("codex").unwrap();
        assert!(codex.appendable.is_supported());
    }

    #[test]
    fn codex_replace_reason_names_the_unsent_base_channel() {
        let codex = provider_base_prompt_authority("codex").unwrap();
        let reason = codex.replaceable.reason().unwrap();
        assert!(reason.contains("baseInstructions"), "{reason}");
    }

    #[test]
    fn opencode_reasons_name_the_per_turn_reconstruction() {
        let opencode = provider_base_prompt_authority("opencode").unwrap();
        for verdict in [opencode.appendable, opencode.replaceable] {
            let reason = verdict.reason().unwrap();
            assert!(reason.contains("every turn"), "{reason}");
        }
    }

    // -----------------------------------------------------------------------
    // No capability is enabled from an assumption.
    // -----------------------------------------------------------------------

    #[test]
    fn no_capability_is_enabled_from_an_assumption() {
        // The acceptance bar for this slice: nothing here claims `Supported`
        // unless a piece of shipped code or a recorded measurement can be
        // pointed to as the proof. Only Claude's append (options.mjs's
        // preset+append call) and Codex's append (the prefix-cache experiment
        // in docs/managed-agent-runtimes.md) are proven today. Everything else
        // pins `Unsupported` until it gains a citation of its own.
        let supported_count = provider_base_prompt_authorities()
            .iter()
            .flat_map(|capability| [capability.readable, capability.appendable, capability.replaceable])
            .filter(|verdict| verdict.is_supported())
            .count();
        assert_eq!(supported_count, 2, "only Claude's and Codex's append are proven today");
    }
}
