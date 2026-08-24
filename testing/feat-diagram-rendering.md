# feat/diagram-rendering — Test Contract

## Functional Behavior

- A ` ```diagram ` fenced code block containing a JSON `DiagramSpec` renders as a hand-drawn node/edge figure with a caption, matching Bridge's own visual language (achromatic by default, a single accent color reserved for the one thing that matters, no drop shadows or blur).
- The figure re-themes for free: every stroke/fill/text is `currentColor` or `var(--ring)`, both of which already flip with the `dark` class, so there is no theme-tracking effect and no re-render-on-theme-change needed (unlike the Mermaid path it replaces).
- An invalid or malformed spec (bad JSON, missing required fields, an edge referencing a node id that doesn't exist) falls back to a plain labeled code block showing the raw source, exactly like the existing Mermaid-parse-failure fallback — never a crash.
- Mermaid is fully removed: the `mermaid` npm dependency, the `MermaidBlock` component, and the `` ```mermaid `` fenced-block special case are all gone. An old ` ```mermaid ` block now falls through to an ordinary labeled code block (safe, confirmed degradation — no special-cased deprecation message).
- The orchestrator's `RENDERING_NOTE` system-prompt fragment advertises the `` ```diagram `` format (shape of `DiagramSpec`, what `row`/`col`/`emphasis`/`marker`/`curve` mean) instead of Mermaid, so agents actually emit it.
- The raw JSON source is copyable via the same `rich-block`/`rich-block-copy` affordance used by math and (formerly) Mermaid blocks.

## Unit Tests

- `DiagramFigure.test.tsx`: `layoutDiagram` grid math (row/col → x/y, viewBox sizing, negative columns, right-label budget, below-label budget, `continues` marker budget); `isValidDiagramSpec` accepts a well-formed spec and rejects each failure mode (missing caption/ariaLabel, empty nodes, duplicate node ids, edge referencing an unknown node id, bad enum values); component rendering smoke tests (node count, halo presence for `checkpoint`/`tip`, accent color only on `active`-emphasis marks, figcaption text).
- `Markdown.test.tsx`: detects a `` ```diagram `` fenced block into the new `Block` kind; renders a valid spec to an `<svg role="img">` with the right `aria-label`; falls back to a labeled code block on invalid JSON and on a spec with a dangling edge reference; an old `` ```mermaid `` block now renders as a plain code block, not a diagram or a crash; copy button on a diagram block copies the raw JSON source (mirrors the removed Mermaid copy test).
- Rust `prompts.rs` tests: `rendering_note_lists_every_supported_format` checks for `` ```diagram `` instead of `` ```mermaid ``; `REQUIRED_MARKERS` lint no longer references Mermaid.

## Integration / Functional Tests

- `bun run check` passes (`tsc -b` + `cargo check --workspace`) with `mermaid` removed from `package.json` and no dangling imports.
- `bun run test` passes end to end (sidecar + vitest + cargo test).
- `bun run build` completes with no Mermaid chunk in the output bundle.

## Smoke Tests

- A gallery of at least 10 varied `DiagramSpec` examples (linear history, fork, comparison, layered architecture, pipeline/approval flow, etc.) all render without console errors in both light and dark themes.

## E2E Tests

N/A — no automated desktop E2E harness is configured for this change.

## Manual / cURL Tests

N/A — this is an in-app renderer change with no HTTP endpoint.
