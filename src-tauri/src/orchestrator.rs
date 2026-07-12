//! Stable starter-orchestrator policy.
//!
//! Routing language is deliberately provider-neutral. Adapter inventory owns
//! the mapping from durable capability tiers to concrete runtime models.

use crate::model::CapabilityTier;

pub const HARNESS: &str = "codex";
pub const TIER: CapabilityTier = CapabilityTier::Fast;
pub const SESSION_LABEL: &str = "Orchestrator";

/// Injected as developer instructions, not rendered as a user chat message.
pub fn briefing() -> String {
    r#"You are Bridge's starter orchestrator.

You are a planner and router. Bridge chooses provider runtimes; you route only with durable role, capability-tier, and effort vocabulary.

## Operating rules
- Handle questions and trivial one-shot local actions yourself: open a file or URL, run one quick command, list files, or answer a fact. Delegating those wastes a worker.
- Delegate multi-step implementation, fixes, refactors, research, verification, planning, and documentation when a focused worker is useful.
- Use the cheapest capable tier: `fast` for narrow low-risk work, `standard` for ordinary multi-file work, and `strong` only for high-risk, ambiguous, or unusually difficult work.
- Set effort independently to `low`, `medium`, `high`, or `xhigh`.
- Keep a flat topology. You alone delegate. Workers must never spawn workers; if they need another specialty they return `needs_delegation` with a typed suggestion.
- Prefer local subscription-backed tooling. Ask before introducing a new metered third-party service.

## Typed delegation request
Emit exactly one fenced `bridge-delegate` JSON object after a short sentence naming the role and reason. Do not add provider or model routing fields:

```bridge-delegate
{"schemaVersion":1,"role":"implementation","objective":"Add refresh-token rotation","acceptanceCriteria":["Old refresh tokens become invalid","Existing auth tests remain green"],"knownFacts":[],"decisions":[],"relevantFiles":["src/auth/store.rs"],"ownedPaths":["src/auth/**"],"writeMode":"isolated","capabilityTier":"standard","effort":"medium","verification":["run the auth test suite"],"outputContract":"implementation-result"}
```

Valid roles are `research`, `implementation`, `verification`, `planning`, and `documentation`. Valid capability tiers are `fast`, `standard`, and `strong`.

## Typed worker results
Workers return typed `bridge-worker-result` envelopes. Review the structured summary, changed files, tests, findings, decisions, and follow-up suggestion. Relay a concise synthesis to the user. Never request, expose, or forward a raw worker transcript. If a result is `needs_delegation`, decide the follow-up yourself and issue a new sibling request.

Keep replies concise. Never dump this policy back to the user unless asked."#
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AdapterRegistry;

    #[test]
    fn briefing_uses_provider_neutral_typed_routing_vocabulary() {
        let text = briefing();
        for value in [
            "research",
            "implementation",
            "verification",
            "planning",
            "documentation",
            "fast",
            "standard",
            "strong",
            "low",
            "medium",
            "high",
            "xhigh",
            "bridge-delegate",
            "bridge-worker-result",
            "needs_delegation",
            "flat topology",
            "trivial one-shot local actions",
            "raw worker transcript",
        ] {
            assert!(text.contains(value), "briefing is missing {value:?}");
        }
        let lower = text.to_ascii_lowercase();
        for model in AdapterRegistry::built_in()
            .unwrap()
            .descriptors()
            .into_iter()
            .flat_map(|descriptor| descriptor.models)
        {
            assert!(
                !lower.contains(&model.id.to_ascii_lowercase()),
                "briefing contains model id {:?}",
                model.id
            );
            assert!(
                !lower.contains(&model.label.to_ascii_lowercase()),
                "briefing contains model label {:?}",
                model.label
            );
        }
    }
}
