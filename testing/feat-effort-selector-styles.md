# feat/effort-selector-styles — Test Contract

The model picker's effort footer is a `flex-1` segmented row with fixed padding.
It was sized for three to five short labels; Claude's ladder now ends at
`xhigh`/`max` and Codex's at `ultra`, so six items squeeze inside the 340px
popover and the row jitters, wraps, or clips. Card-picking effort on a model you
are only browsing is impossible: the footer follows the selected model.

This branch replaces the footer with a user-chosen **thinking control style**,
persisted like the theme skin and picked in Settings › Appearance:

- **Slider** (default) — a rail with one tick per level, a thumb, an accent fill
  in the harness's own tint, one label at a time. Animated on change.
- **Sentence** — the footer reads "Think *deeply* with Sonnet 5." The word is a
  scrub token: drag sideways, click to step, arrow keys. Letters scramble
  briefly on change.
- **List** — the popover becomes two panes: models on the left, effort as a
  vertical radio list with a one-line meaning per level on the right. Picking a
  model keeps the popover open so effort can be set for it next.

Locked before implementation.

---

## Functional Behavior

### Preference (`src/theme.ts`)
- New `EffortSelectorStyle = "slider" | "sentence" | "list"`, stored under
  `bridge.effortSelector`. Default `slider`. Corrupt or throwing storage falls
  back to the default and never throws on write.
- `useThemePreference()` additionally returns `effortSelector` /
  `setEffortSelector`. A lightweight `useEffortSelectorStyle()` hook exposes the
  value to the picker without re-applying theme or skin, and follows the same
  `bridge:theme` broadcast so Settings and an open picker stay in sync.

### Shared effort footer rules (every style)
- The footer is a fixed-height box (`h-[72px]`, `contain: layout`) for slider
  and sentence; its size never depends on the level count or the label.
- Exactly one visible label at a time, pinned width (`min-w-[6ch]` tabular).
  Per-level marks are unlabeled buttons that `flex-1`/absolute-position; each
  carries `data-effort`, `aria-pressed`, and a screen-reader-only label so the
  existing footer contract (`[data-testid="effort-control"] button`) still holds.
- Levels come only from `supportedEffortLevels` of the resolved model. No level
  is invented; no footer when the model has none or discovery reported nothing.
- The footer is inert (buttons disabled) when the control is `disabled` or no
  `onEffortChange` is wired; it still shows the current value.
- Only `transform`, `opacity`, `width`, and colour are animated; nothing moves
  the layout. Idle motion is none. `prefers-reduced-motion` is honoured by the
  global freeze plus the sentence scramble skipping itself.
- Accent is the harness tint token (`text-harness-*` / `bg-harness-*`); no new
  colour.

### Slider (`src/components/effort/EffortSlider.tsx`)
- Header row: "Thinking" + current label. Rail beneath with a fill from 0 to the
  current tick, a thumb, and one tick per level.
- Click on rail or a tick, drag the thumb (pointer capture), or use arrow keys /
  Home / End on the thumb (`role="slider"`, `aria-valuenow` = index,
  `aria-valuetext` = label) → `onEffortChange(value)`.
- On change: fill width eases, thumb pops (scale pulse), label fades in, ticks
  at or below the value light in the harness tint with a small stagger.
- Five (Claude) and six (Codex) levels render in the same footer box.

### Sentence (`src/components/effort/EffortSentence.tsx`)
- Reads "Think **<word>** with <model label>." Word by wire value:
  low→lightly, medium→briefly, high→properly, xhigh→deeply,
  max→exhaustively, ultra→relentlessly; an unknown value shows itself.
- Word is a button with `min-w-[12ch]`. Click steps to the next level and wraps.
  Pointer drag: one step per 28px. ArrowLeft/ArrowRight step. A tick row
  beneath mirrors the slider's marks.
- Scramble: letters randomise then settle over ~260ms; the final text always
  equals the target word; timers are cleared on unmount; skipped under reduced
  motion.

### List (`src/components/effort/EffortList.tsx` + two-pane popover)
- With list style and an effort-capable surface, the popover is `w-[460px]`
  with the model list left (`w-[210px]`) and a pane right showing the resolved
  model's harness mark, its label, and a `role="radiogroup"` of levels: label,
  meaning, and digit shortcut. Digits 1–9 while the pane has focus select.
- Selecting a model in list style calls `onChange` and keeps the popover open.
  Other styles keep today's close-on-select.
- A model with no effort levels shows "No thinking control for this model."
  in the pane instead of hiding the pane, so the popover width is stable across
  models. When the surface wires neither `effort` nor `onEffortChange`, the
  popover stays single-pane.

### Settings › Appearance (`src/components/settings/AppearancePage.tsx`)
- Third group "Thinking control" with three tiles (Slider / Sentence / List),
  each with a small static preview instead of swatches. Selecting one persists
  the style and broadcasts. Search index gains an entry for the group.

## Unit Tests
- `theme.test.ts`: `isEffortSelectorStyle` accepts the three, rejects others;
  `readEffortSelectorStyle` defaults, reads, falls back on corrupt value and on
  throwing storage; `writeEffortSelectorStyle` persists and swallows throws.
- `effortLevels.test.ts`: `effortLabel`, `effortWord`, `effortMeaning`
  fallbacks for unknown wire values.

## Integration / Functional Tests (`ChatModelControl.test.tsx`, jsdom)
- All existing effort tests pass unchanged under the default slider.
- Slider: `role="slider"` has `aria-valuetext="High"`; ArrowRight →
  `onEffortChange("xhigh")`; Home → `"low"`; six Codex levels render six
  `[data-effort]` marks; the footer box has the fixed-height class.
- Sentence (style pre-stored in localStorage): word reads "properly" for
  `high`; click → `onEffortChange("xhigh")`; at the top level click wraps to the
  first; ArrowLeft → previous; the model label appears in the sentence.
- List: radios render with `aria-checked` on the current; clicking a radio
  fires `onEffortChange`; clicking a model row calls `onChange` and the popover
  stays open; a no-effort model shows the "No thinking control" copy; a
  surface with neither effort prop is single-pane.
- `SettingsScreen.test.tsx`: Appearance shows "Thinking control"; clicking
  "Sentence" stores `sentence` under `bridge.effortSelector`.

## Smoke Tests
- `bun run build` and `bunx vitest run` green.
- `bun run dev`, open the composer picker: slider footer renders for a
  thinking-capable model; Settings › Appearance › Thinking control swaps the
  footer live without reopening the app.

## E2E Tests
N/A — no wire change; frontend-only.

## Manual Tests
1. Pick a Claude model with `max` and a Codex model with `ultra`; the footer
   box does not change height or width between them in slider and sentence.
2. Drag the slider thumb; the fill follows and the thumb pops on each tick.
3. Sentence: drag the word right three steps; the letters scramble then settle.
4. List: click a different model; the popover stays open and the right pane
   now names that model; press `3`; the third level is chosen.
5. Enable "Reduce motion" in macOS; the sentence changes without scrambling.
