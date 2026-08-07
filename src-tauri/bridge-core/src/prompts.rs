//! Shared prompt fragments injected into every Bridge agent surface.
//!
//! Kept provider-neutral: this describes Bridge's own rendering capabilities,
//! not any particular runtime.

/// Tells every agent (orchestrator and workers) which rich-content formats
/// Bridge renders inline in its chat UI, so they can choose to emit them.
pub const RENDERING_NOTE: &str = "## Rich rendering in the Bridge chat UI
Bridge renders your replies inline — no external or headless browser is involved:
- Mermaid diagrams: put the diagram in a ```mermaid fenced code block.
- Math / LaTeX: use `$...$` for inline math and `$$...$$` (or a ```math fenced block) for display math.
- HTML: put markup in a ```html fenced code block; it renders in a fully sandboxed iframe (no scripts run), so treat it as layout, not a live app.
Reach for these when a diagram, formula, or formatted layout communicates better than plain prose; otherwise keep replies in plain markdown.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_note_lists_every_supported_format() {
        for value in ["```mermaid", "$$", "```math", "```html", "sandboxed"] {
            assert!(
                RENDERING_NOTE.contains(value),
                "rendering note is missing {value:?}"
            );
        }
    }
}
