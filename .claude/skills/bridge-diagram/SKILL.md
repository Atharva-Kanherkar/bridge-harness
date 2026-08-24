---
name: bridge-diagram
description: Draw a diagram in Bridge's own visual language — a small hand-drawn node/edge grid, achromatic except for one accent color — instead of a generic Mermaid-style box-and-arrow diagram. Use for any diagram inside Bridge chat (a ```diagram fenced block), the Bridge docs site, or a Bridge PR/README that needs to show a real mechanism.
---

# Bridge diagram style

Bridge does not render Mermaid. Every diagram — in the chat UI, the docs site, or
a PR description — is a small **node/edge grid**, drawn either as a `DiagramSpec`
JSON block (the in-app renderer, `src/components/DiagramFigure.tsx`) or as
hand-authored inline SVG following the exact same visual grammar (docs pages,
READMEs, anywhere the React component isn't available). This skill is what to
reach for instead of a generic dark-canvas, gray-box, default-arrow diagram —
that look is what happens when a model can draw anything; this format is
deliberately narrow so it can't produce that.

## When to use this

Any time you're about to draw a diagram for Bridge: explaining a mechanism in
chat, illustrating a docs page, or including a diagram in a PR description.
**Never** reach for Mermaid syntax, a generic flowchart tool, or a screenshot of
some other app's diagram style — draw it in this grammar instead.

## The one rule that matters

**Depict the mechanism, not its name.** Show the nodes and edges the argument
actually hinges on — a fork, a gate, a fan-in — not a box labeled "system." If a
sentence says it faster, don't draw it. Match complexity to the stakes: a
one-hop relationship is 3 nodes; a real pipeline gets every stage it actually
has, no more.

## Visual grammar

- **Achromatic by default, one accent for meaning.** Every neutral mark is
  `currentColor` (so it repaints for free on the `dark` class flip — no
  theme-tracking code needed, unlike Mermaid). Exactly one hue exists —
  Bridge's `--ring` slate blue — reserved for the single thing the reader
  should follow (the active branch, the approved path, the routed choice).
  Never use it decoratively, and never add a second hue.
- **Grid, not auto-layout.** Nodes sit on `row`/`col` integers — row grows
  downward, col grows rightward from 0. There is no force-directed layout to
  fight; you place every node deliberately.
- **Fork points draw themselves heavier.** A node's out-degree (how many edges
  leave it) is computed, not authored — any node with more than one outgoing
  edge renders at a slightly larger radius automatically.
- **Muted means "still in the forest, not the point."** An abandoned branch or
  a denied/failed path never disappears — it renders at reduced opacity
  (`emphasis: "muted"`), never deleted from the picture. That's the actual
  Bridge philosophy (append-only, nothing rewritten), not just a style choice.
- **Markers carry state, not color.** `marker: "checkpoint"` draws a solid halo
  ring; `marker: "tip"` draws a dashed halo (something still live/provisional);
  `marker: "continues"` draws three fading dots suggesting growth past this
  point. These are shape distinctions, not new colors.
- **Curved edges peel off; straight edges continue.** `curve: true` draws a
  smooth S-curve to a node in a different column (a branch, a fan-out/fan-in).
  A straight line means "the same line of history, continuing."
- **Label placement is a request, not a guarantee.** A `label` renders to the
  `right` of its node by default (a short tick, then text), or `below`,
  centered, when you ask for it — but the renderer will override either
  choice to keep the result legible (see below). Author for the common case;
  don't hand-tune around edge cases the renderer already handles.

## What the renderer guarantees, so you don't have to

Three live tests against the shipped renderer found real ways a legible-looking
spec still rendered as a mess. Each is now a structural guarantee, not an
authoring rule — but understanding them explains why some specs render
differently than their `row`/`col` values might suggest:

1. **No same-row collision, regardless of authored spacing.** Column pitch
   isn't a fixed 66px — it widens for the whole diagram to whatever its widest
   label needs, so three labeled nodes at `col: 0, 1, 2` will never crowd each
   other. You never need to manually space siblings apart; a short label
   costs nothing, and one long label among short ones widens the whole grid
   rather than only the pair that needs it.
2. **No label ever sits under an edge that crosses it.** This was the subtler
   bug: a right-side label sits at its node's own y — exactly where a
   *horizontal* edge to a same-row neighbor runs, so the edge struck through
   the text no matter how wide the columns were. Wider spacing doesn't fix a
   problem on the wrong axis. Any label a same-row edge would cross now drops
   below the line automatically. Practical effect: in a **horizontal chain**
   (nodes connected left-to-right, edges running along their row), every
   label except the last one typically renders below the line — that's
   correct, not a fallback to apologize for.
3. **The drawing displays larger than its native geometry.** Labels are drawn
   at 11px in the SVG's own coordinate space, which reads smaller than
   Bridge's 15px chat prose at 1:1 scale. The renderer displays the whole
   figure at 1.35x so labels land near prose size — you don't need to inflate
   anything yourself.

Keep labels to 2-3 words regardless of all of the above — a label wider than
roughly 12 characters is a sign the node needs a shorter name, not a system
that will keep finding more space for it. `truncateLabel` caps at 18
characters with an ellipsis as a hard backstop, not a target to write to.

## The DiagramSpec shape

```json
{
  "nodes": [
    { "id": "fork", "row": 2, "col": 0, "label": "fork" },
    { "id": "abandoned", "row": 3, "col": -1, "label": "abandoned", "labelSide": "below", "emphasis": "muted" },
    { "id": "tip", "row": 5, "col": 0, "label": "active tip", "emphasis": "active", "marker": "tip" }
  ],
  "edges": [
    { "from": "fork", "to": "abandoned", "curve": true, "emphasis": "muted" },
    { "from": "fork", "to": "tip", "emphasis": "active" }
  ],
  "caption": "The one sentence this figure proves.",
  "ariaLabel": "A plain-language equivalent of the same claim, for screen readers."
}
```

Field reference:
- **node**: `id` (unique string), `row`/`col` (integers), `label?`, `emphasis?`
  (`"default" | "muted" | "active"`, default `"default"`), `marker?`
  (`"none" | "checkpoint" | "tip" | "continues"`, default `"none"`),
  `labelSide?` (`"right" | "below"`, default `"right"`).
- **edge**: `from`/`to` (node ids — both must exist), `curve?` (boolean),
  `emphasis?` (same enum as above).
- **`caption`** is the one claim the figure supports — it renders under the
  figure. **`ariaLabel`** is that same claim in prose, for `role="img"`.

## Output format by context

- **Inside Bridge chat** (an agent's own reply): a ` ```diagram ` fenced code
  block containing exactly this JSON. Bridge's `Markdown.tsx` parses and
  renders it; a malformed spec falls back to a plain code block, never a crash.
- **Docs site / README / anywhere without the React renderer**: hand-author
  inline SVG that reproduces the same grammar directly — circles for nodes
  (`r="5"`, `r="6"` for heavy/fork nodes), `currentColor`/`var(--ring)` fills,
  halo rings (`r="11"`, solid for checkpoint, `stroke-dasharray="2.5 3"` for
  tip), 56px row spacing — wrapped in
  `<figure><svg role="img" aria-label="…">…</svg><figcaption>…</figcaption></figure>`.
  Column spacing and label placement aren't fixed constants in the real
  renderer (see "What the renderer guarantees" above) — when hand-authoring,
  just apply that same judgment directly: widen columns for long labels, and
  drop a label below the line if a horizontal edge would run through it.
  See `src/components/DiagramFigure.tsx` for the canonical logic to mirror.
- **A PR description**: screenshot the rendered figure (light and dark) rather
  than pasting raw JSON — reviewers judge diagrams by eye, not by schema.

## Worked reference

`src/components/DiagramFigure.tsx` is the source of truth for the exact pixel
constants (padding, radii, halo size, font). `src/diagram-gallery-entry.tsx`
(generated transiently for PR evidence, not shipped) held 10 worked examples
spanning a linear chain, a fork with an abandoned branch, fan-out, fan-out/fan-in,
multiple checkpoints, a terminal-failure branch, fan-in from siblings, a bare
pipeline, a backward-referencing resume curve, and a side-by-side comparison —
recreate any of these shapes as a starting point rather than inventing a new
topology from scratch.
