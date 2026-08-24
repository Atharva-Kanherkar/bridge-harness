//! Bridge-owned prompt defaults and shared prompt vocabulary.
//!
//! Kept provider-neutral: this describes Bridge's own rendering capabilities,
//! not any particular runtime.

use crate::{
    delegation::{self, WorkerRole},
    orchestrator,
};

pub const BRIDGE_ROLE_SECTION_ID: &str = "bridge_role";
pub const DELEGATION_PROTOCOL_SECTION_ID: &str = "delegation_protocol";
pub const WORKER_CONTRACT_SECTION_ID: &str = "worker_contract";

pub const ORCHESTRATOR_SECTION_IDS: &[&str] =
    &[BRIDGE_ROLE_SECTION_ID, DELEGATION_PROTOCOL_SECTION_ID];
pub const WORKER_SECTION_IDS: &[&str] = &[WORKER_CONTRACT_SECTION_ID];
pub const DIRECT_SESSION_SECTION_IDS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTarget {
    Orchestrator,
    Worker(WorkerRole),
    DirectSession,
}

impl PromptTarget {
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Worker(WorkerRole::Research) => "worker:research",
            Self::Worker(WorkerRole::Implementation) => "worker:implementation",
            Self::Worker(WorkerRole::Verification) => "worker:verification",
            Self::Worker(WorkerRole::Planning) => "worker:planning",
            Self::Worker(WorkerRole::Documentation) => "worker:documentation",
            Self::DirectSession => "direct_session",
        }
    }

    pub const fn section_ids(self) -> &'static [&'static str] {
        match self {
            Self::Orchestrator => ORCHESTRATOR_SECTION_IDS,
            Self::Worker(_) => WORKER_SECTION_IDS,
            Self::DirectSession => DIRECT_SESSION_SECTION_IDS,
        }
    }

    pub fn compiler_role(self) -> String {
        match self {
            Self::Orchestrator => "orchestrator".into(),
            Self::Worker(role) => format!("worker:{}", role.as_str()),
            Self::DirectSession => "session".into(),
        }
    }
}

pub const PROMPT_TARGETS: &[PromptTarget] = &[
    PromptTarget::Orchestrator,
    PromptTarget::Worker(WorkerRole::Research),
    PromptTarget::Worker(WorkerRole::Implementation),
    PromptTarget::Worker(WorkerRole::Verification),
    PromptTarget::Worker(WorkerRole::Planning),
    PromptTarget::Worker(WorkerRole::Documentation),
    PromptTarget::DirectSession,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDefaultSection {
    pub id: &'static str,
    pub text: String,
}

/// Returns the stable Bridge-authored sections used by the current live path.
/// Worker defaults remain depth-sensitive because the contract embeds topology.
pub fn default_sections(target: PromptTarget, worker_depth: i64) -> Vec<PromptDefaultSection> {
    match target {
        PromptTarget::Orchestrator => vec![
            PromptDefaultSection {
                id: BRIDGE_ROLE_SECTION_ID,
                text: orchestrator::briefing(),
            },
            PromptDefaultSection {
                id: DELEGATION_PROTOCOL_SECTION_ID,
                text: delegation::protocol(0),
            },
        ],
        PromptTarget::Worker(role) => vec![PromptDefaultSection {
            id: WORKER_CONTRACT_SECTION_ID,
            text: delegation::worker_contract(role, worker_depth),
        }],
        PromptTarget::DirectSession => Vec::new(),
    }
}

pub const REQUIRED_MARKERS: &[&str] = &[
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
    "bridge-steer",
    "needs_delegation",
    "flat topology",
    "trivial one-shot local actions",
    "raw worker transcript",
    "readOnly",
    "isolated",
    "shared",
    "full",
    "research-result",
    "implementation-result",
    "there is no `none`",
    "structured MCP/API",
    "attached authenticated tab",
    "local headless browser",
    "optional remote browser",
    "screenshot-first computer use",
    "automated_test",
    "untrusted evidence",
    "```diagram",
    "sandboxed iframe",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLintWarning {
    pub marker: &'static str,
    pub message: String,
}

pub fn lint_required_markers(text: &str) -> Vec<PromptLintWarning> {
    REQUIRED_MARKERS
        .iter()
        .filter(|marker| {
            if **marker == "bridge-delegate" {
                !text.contains("```bridge-delegate")
            } else {
                !text.contains(**marker)
            }
        })
        .map(|marker| PromptLintWarning {
            marker,
            message: if *marker == "bridge-delegate" {
                "Typed delegation may stop working because `bridge-delegate` is missing.".into()
            } else {
                format!(
                    "Bridge behavior that depends on the required marker `{marker}` may stop working."
                )
            },
        })
        .collect()
}

/// Tells an agent which rich-content formats Bridge renders inline in chat.
/// It is currently embedded in the orchestrator default and intentionally not
/// injected into the live worker path.
pub const RENDERING_NOTE: &str = "## Rich rendering in the Bridge chat UI
Bridge renders your replies inline — no external or headless browser is involved:
- Diagrams: put a JSON spec in a ```diagram fenced code block (Bridge does not render Mermaid). Shape: nodes (id, row, col, optional label/emphasis/marker/labelSide) and edges (from, to, optional curve/emphasis), plus a caption and an ariaLabel. row/col place nodes on a grid; emphasis: active is the one accent color a diagram gets, so reserve it for whatever the reader should follow; marker: checkpoint or tip draws a halo ring; curve: true peels an edge off to the side instead of a straight line. Keep it small — a few nodes that show one real mechanism, not an inventory.
- Math / LaTeX: use `$...$` for inline math and `$$...$$` (or a ```math fenced block) for display math.
- HTML: put markup in a ```html fenced code block; it renders in a fully sandboxed iframe (no scripts run), so treat it as layout, not a live app.
Reach for these when a diagram, formula, or formatted layout communicates better than plain prose; otherwise keep replies in plain markdown.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_note_lists_every_supported_format() {
        for value in ["```diagram", "$$", "```math", "```html", "sandboxed"] {
            assert!(
                RENDERING_NOTE.contains(value),
                "rendering note is missing {value:?}"
            );
        }
    }

    #[test]
    fn target_inventories_match_the_live_prompt_shapes() {
        assert_eq!(
            PromptTarget::Orchestrator.section_ids(),
            [BRIDGE_ROLE_SECTION_ID, DELEGATION_PROTOCOL_SECTION_ID]
        );
        for target in PROMPT_TARGETS
            .iter()
            .copied()
            .filter(|target| matches!(target, PromptTarget::Worker(_)))
        {
            assert_eq!(target.section_ids(), [WORKER_CONTRACT_SECTION_ID]);
            let defaults = default_sections(target, 1);
            assert_eq!(defaults.len(), 1);
            assert!(!defaults[0].text.contains(RENDERING_NOTE));
        }
        assert!(PromptTarget::DirectSession.section_ids().is_empty());
        assert!(default_sections(PromptTarget::DirectSession, 0).is_empty());
    }

    #[test]
    fn required_marker_lint_agrees_with_the_default_briefing() {
        let default = orchestrator::briefing();
        assert!(lint_required_markers(&default).is_empty());

        let fence_start = default.find("```bridge-delegate").unwrap();
        let fence_end = default[fence_start + 3..]
            .find("```")
            .map(|offset| fence_start + 3 + offset + 3)
            .unwrap();
        let mut edited = default.clone();
        edited.replace_range(fence_start..fence_end, "");
        assert!(edited.contains("bridge-delegate"));
        let warnings = lint_required_markers(&edited);
        let delegation = warnings
            .iter()
            .find(|warning| warning.marker == "bridge-delegate")
            .expect("removing bridge-delegate produces a warning");
        assert!(delegation.message.contains("Typed delegation"));
    }
}
