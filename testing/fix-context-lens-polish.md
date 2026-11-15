# fix/context-lens-polish — Test Contract

The Context lens modal clips its right edge whenever a chat has more than one
window, and its look (rainbow bar, hand-rolled pills) does not match Bridge's
achromatic chrome. This change fixes the layout at every size and restyles it.
No protocol, Rust or data change: every number shown before is still shown.

## Functional Behavior

- With several windows, no element inside the dialog extends past the dialog's
  right edge at 380, 560, 1100, 1440 and 2560 px viewport widths.
- The window list never scrolls sideways. Below 48rem of panel width it is a
  wrapping grid above the detail; at 48rem and wider it is a rail on the left.
- The dialog is `max-w-2xl` for a single window and widens to `lg:max-w-4xl`
  when the chat has more than one window.
- The window list keeps `role="tablist"` / `role="tab"` / `aria-selected`, and
  is still hidden when the chat is the only window.
- Segments in the composition bar, legend and "What fills it" rows wear an ink
  ramp (one neutral ink, ranked strongest first), not `bg-ctx-*` hues.
  Unattributed space keeps the neutral hatch.
- Pressure colour is unchanged: the hero card washes amber at high and red at
  critical; healthy and elevated stay neutral.
- Compact lives in the hero card's footer, as a kit `Button`. Its label,
  `aria-label`, disabled states, notes and failure alert are unchanged.
- State (measured, reported, estimated) and pressure are kit `Badge`s.
- Everything previously shown is still shown: percent, used/window tokens,
  free, auto-compact marker and label, legend, forecast, consumers, Bridge's
  share, earlier windows, compaction owner note, unavailable reason, the
  loading skeleton and the daemon-unavailable alert.

## Unit Tests

`src/components/ContextLensDialog.test.tsx` (all existing cases keep passing,
adjusted only where they assert the old structure):

- one tab per window, honest tab for a window that cannot report
- pressure hero, composition, consumers, forecast, Bridge's share, earlier
- switches windows from the tabs
- opens on the requested window
- hides the tab strip when the chat is the only window
- stops polling when the daemon lacks the method
- does not fetch while closed
- compacts through the harness or says Bridge checkpoints
- shows why a compaction was refused
- closes through the dialog

New cases:

- "lays the window list out as a wrapping grid that becomes a rail, never a sideways scroller"
- "widens the dialog only when there is more than one window"
- "draws segments in the ink ramp, strongest first, with no categorical hue"
- "puts Compact inside the pressure card as a kit button"

`src/designSystem.test.ts` stays green.

## Integration / Functional Tests

N/A. One presentational component; its data hook is untouched.

## Smoke Tests

- `bunx tsc -b` clean.
- `bun run build` green.
- `bunx vitest run` green.

## E2E Tests

Headless Chrome over CDP against the real component with fixture data
(throwaway Vite entry, not committed): for each of 380x560, 560x640, 1100x520,
1440x900, 2560x1440, in dark and light, assert zero descendants of the dialog
whose right edge exceeds the dialog's right edge, and capture a screenshot.

## Manual / cURL Tests

`bun run dev`, open a chat with workers, click the context ring in the
composer. Resize the window from narrow to wide: the list goes from a grid on
top to a rail on the left, and nothing is cut off.
