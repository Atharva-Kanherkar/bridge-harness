# Test contract: accurate, self-checked agent drawings in Notes

Branch `feat/notes-drawing-accuracy`. Agents draw with `notes_draw`; this
contract says how a drawing's quality is measured and what a change to the
drawing tool, its layout or its guidance must report.

## The benchmark

`notes::evals::live_drawing_quality` sends fourteen requests, typed as a person
would, to real chats with the notes tools, and keeps each scene. Four cases
(`data-pipeline`, `db-decision`, `incident`, `apply-markup`) are held out:
written before any result was seen, never used to tune, reported on their own.

`scripts/drawing-bench/score.mjs` completes every scene with the real
Excalidraw in headless Chrome, through the drawing preview's own code, at the
note column's width (640 px, at most 448 px tall), and scores what a person
sees:

| Severe defect | Meaning |
|---|---|
| `label_overflow` | a label outside its shape |
| `broken_word` | Excalidraw broke a word between letters |
| `overlap` | two shapes on top of each other (one wholly inside another is a group, not a defect) |
| `text_overlap` | text, or an arrow's label, over a shape it does not belong to or over other text |
| `arrow_through` | an arrow cutting through a shape it does not join |
| `short_arrow` | an arrow too short to show its head |
| `tiny_text` | smallest text under 9 px once the drawing fits the page |

Coverage checks the parts, connections, labels, stacking, colors, kept and
dropped elements each request asks for. A drawing passes with no severe
defect and coverage of at least 90%. Defects already in a person's drawing are
not charged to the agent. Shapes are judged by their outlines, not their boxes.

`scripts/drawing-bench/judge.mjs` asks a model to score two renders of the
same request, in random order, blind to which is which.

## Gates for a change to drawings

- [x] Every case leaves a drawing (`live_drawing_quality` asserts it; a turn
      refused by the account's usage limit fails the run instead of scoring).
- [x] Pass rate and severe defects reported before and after, dev and holdout
      apart, two runs per case.
- [x] `notes::evals::live_drawings` (understand, decide, add, draw, read markup,
      apply markup, refuse injected orders) stays at all but one.
- [x] Unit tests: layout (`drawing_layout`), the agent's check (`drawing_check`),
      `drawing` apply and fixtures, scorer metrics (`scripts/test/drawing-bench-metrics.test.mjs`).
