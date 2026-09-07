# feat/prompt-studio-redesign — Test Contract

Reskins Prompt Studio (Settings → Prompt Studio) per #393 direction 1
("Manuscript"): one rail + one document column, typographic hierarchy,
history as a horizontal timeline, compilation demoted to a quiet receipt.
Implements #393; supersedes the layout shipped by feat/prompt-studio-4-interface.

**Scope guard: this is a reskin of `src/components/PromptStudio.tsx`'s markup
only. No API, storage, protocol, or state-model changes. Every functional
behavior of the 4-interface contract carries over unchanged.**

## Functional Behavior

### Carryover (unchanged from testing/feat-prompt-studio-4-interface.md)

- Target/section navigation via `bridgeApi.promptStack(target)`; section rows
  show id, live token estimate, modified/deleted badges, unsaved-draft dot.
- `direct_session` empty state copy ("nothing here for Bridge to override"),
  with the compiled preview receipt still rendered beneath it — the receipt
  is not gated on there being an editable section, since a direct session's
  provider-layer breakdown is real information even with no Bridge overrides.
- CodeEditor reuse with `docKey`/revision-count reseed semantics; drafts
  survive navigation; save via button or Mod-s; reset per section and per
  target (with confirmation); revision history with restore.
- Lint warnings before save (marker + consequence, warn-explain-allow).
- Compiled preview honesty rules (exact Bridge bytes vs provider layers,
  `unavailable` details, prefix-changed indicator, "estimated" wording).
- Export/import of overrides with the same validation and error copy.
- Browser-mode mock parity; no test may require the Tauri runtime.

### Layout contract (the actual change)

- Exactly two persistent regions: a left rail (targets + the active target's
  sections + quiet footer) and a document column. No third or fourth column.
- The rail nests the active target's section listbox under the open target,
  connected by an indent guide; worker roles group under a "Workers" label,
  direct session under "Direct". A target with any non-default section shows
  a status dot on its row.
- The document column reads as a document: breadcrumb (`Target / section_id`),
  state chip (Modified/Deleted), token note, Reset ghost action, inverted
  primary Save, then the lint notice, the editor card (with a quiet file bar
  that carries the unsaved-draft indicator), the horizontal history
  timeline, and the compilation receipt.
- History renders most-recent-first left-to-right as a timeline; each revision
  keeps its Restore action with the same accessible name as before.
- The compilation receipt (exact bytes dl + stable/variable `pre` blocks +
  cache note + provider layers) lives in the same `aside` landmark at the end
  of the document column, collapsible via native `details`, open by default.
  `stablePrefix` remains the first `pre` in the DOM and never contains
  provider-owned detail.
- Export / Import / Reset-all move to the rail footer as quiet actions.

### Test-facing invariants (selectors the existing suite pins)

- `nav[aria-label="Prompt targets"]` wraps the target buttons; each target
  button's exact `textContent` is its label ("Orchestrator", "Research",
  "Direct session", …) — counts/dots add no text inside those buttons.
- Section listbox keeps `role="listbox"` + `aria-label="{Target} prompt
  sections"`; options keep `role="option"`, a `.font-mono` id child, the
  `N tok` estimate, badge words "Modified"/"Deleted", and the
  `aria-label="Unsaved draft"` dot.
- Buttons keep exact texts: "Save {sectionId}", "Reset {sectionId}",
  "Reset all for {targetLabel}", "Export overrides"; the import input keeps
  `aria-label="Import prompt overrides"`; the receipt aside keeps
  `aria-label="Compiled prompt preview and overrides"`.
- Restore buttons keep `aria-label="Restore {sectionId} to revision {id}
  ({operation})"`; the live region keeps `aria-live="polite"` and announces
  saves/failures.

## Unit Tests

`src/components/PromptStudio.test.tsx` — the existing 13 tests pass
**unmodified**. The redesign must not require editing a single assertion;
that is the proof it is a reskin, not a behavior change.

## Integration / Functional Tests

- `src/components/SettingsScreen.test.tsx` — "Prompt Studio" nav item still
  mounts the component; passes unmodified.
- No other suite touches PromptStudio markup.

## Smoke Tests

- `bun run build` passes (tsc strict unused checks + vite).
- `bun run test` passes (vitest + cargo untouched).
- In `bun run dev`: Settings → Prompt Studio renders the two-column layout,
  dark and light themes both read (token-only styling, no hardcoded palette).

## E2E Tests

N/A — component-level coverage plus the browser-mode smoke flow above.

## Manual / cURL Tests

Not applicable (desktop UI change, no network surface). Reviewers verify via
the smoke flow and by diffing `PromptStudio.tsx`'s return JSX against the
mockups in issue #393.
