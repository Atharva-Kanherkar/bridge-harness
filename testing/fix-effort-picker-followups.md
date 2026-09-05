# fix/effort-picker-followups — Test Contract

Three complaints after the thinking-control styles landed:

1. The picker still overflows upward in the chat: a fixed `max-h-[420px]`
   popover opening above a composer near the bottom of a short window runs off
   the top of the viewport.
2. Picking a model and setting its thinking cannot happen in one go. In the
   slider and sentence styles the popover closes on a model pick; in the
   Welcome draft a model pick also wipes the chosen effort.
3. The sentence style's scrub-a-word control is unintuitive. The sentence
   should be the label; a slider should drive it.

Locked before implementation.

## Functional Behavior

### Popover fits the window
- On open (and on window resize while open) the picker measures its trigger.
  Room above = trigger top − 12px; room below = viewport height − trigger
  bottom − 12px. The popover's `maxHeight` is the room on its side, clamped to
  [240, 420]. If its side has under 240px and the other side has more, it flips.
- When the trigger has no box (jsdom, unmounted) nothing is measured and the
  default 420px / requested placement stand.

### One open, both settings
- A model pick keeps the popover open whenever the surface does effort (an
  `effort` value or an `onEffortChange` setter is wired). A surface with
  neither keeps the plain dropdown behaviour: pick closes.
- The picked row highlights immediately and the footer switches to that
  model's ladder, before the host confirms (optimistic `pending` selection,
  cleared when the props catch up, when the host's disable window ends, or
  when the popover closes).
- While the host is still switching (it flips `disabled`), the effort control
  stays enabled; a chosen level is queued and sent the instant the switch
  settles. The queued level is what the control shows meanwhile.
- The disable a picker-initiated change causes never closes the popover; a
  disable from elsewhere (a turn starting) still does.
- Welcome draft: picking a model keeps the chosen effort when the new model's
  ladder includes it, else clears it — mirroring the backend's
  `selected_chat_effort`.

### Sentence rides the slider
- `EffortRail` is extracted from the slider: ticks, fill, thumb, pointer and
  keyboard handling, the pop/halo pulse. Slider = header + rail.
- Sentence = "<Model> thinks **deeply**." as the label line (word scrambles and
  settles on change, mono wire value at the right) + the same rail beneath.
  No draggable word, no click-to-step.

## Unit Tests
- `effortLevels.test.ts`: `supportedEffortLevelsOf` resolves the default model
  when `model` is null and returns [] for an unknown model; `carryEffort`
  keeps a supported level and drops an unsupported one.

## Integration Tests (`ChatModelControl.test.tsx`)
- Slider style: pick another model → popover stays open, that row is
  `aria-selected`, the footer shows the new model's ladder, `onChange` fired.
- Slider style, live host: pick → host disables → slider still enabled → pick
  a level → nothing sent yet, the control shows it → host re-enables → the
  level is sent once.
- No-effort surface: pick still closes.
- Fit: with a mocked trigger box 100px from the top in an 800px window, the
  popover flips down; with 300px above it stays up with `maxHeight: 288px`.
- Sentence: label reads "Claude Sonnet thinks properly."; the rail's slider
  role answers ArrowRight with `xhigh`; unset reads "normally".
- Existing list-style tests keep passing.

## Smoke
- `bun run build`, `bunx vitest run` green. Headless Vite: popover open from
  the composer in a 600px-tall window stays inside the viewport.

## E2E / Manual
N/A beyond the smoke run — frontend-only.
