# Contract: switchable Usage layouts

Branch `feat/usage-layouts`. Locked before implementation. The Usage screen
gains a layout switch with five new presentations of the same
`usage/summary` data, plus the existing screen kept as `Classic`. This is
view-layer only: no protocol change, no new wire method, no new dependency.
The honesty rules in `testing/feat-usage-frontend.md` keep holding in every
layout.

## Surface

1. **Layout switch.** A segmented control labelled `Layout` in the controls
   row offers `Ledger | Strips | Flow | Mosaic | Calendar | Classic`. The
   choice persists in the existing `bridge.usage.preferences.v1` blob as
   `layout`. An unknown or missing value falls back to `ledger` without
   discarding a valid metric or window. Switching layout never refetches.
2. **Ledger.** A large headline figure for the metric, a sentence with the
   harness count, request count and the other metric, one ribbon of harness
   shares, and an itemised receipt grouped by harness: one row per model with
   tokens, cost, and a sparkline of the metric, then a total row. Below it,
   token composition (cache read, cache write, uncached input, output), cache
   savings, and reasoning stated as part of output.
3. **Strips.** A headline row with four facts (requests, share from cache,
   output, cache saved). One strip for all harnesses, then one per harness,
   all on one shared time axis. A `Shared scale | Own scale` toggle sets the
   per-harness y-scale. Hovering or arrow keys move one rule across every
   strip, and a readout lists every harness plus a total for that period. A
   `By model` bar list follows.
4. **Flow.** In token mode, bands run harness → model → token kind, and band
   widths conserve each node's total. In cost mode, bands run harness → model
   and a note says cost cannot be split by token kind. Each harness draws its
   four largest models; the rest share one `N more` band. The drawing grows
   with its label count and labels are spread so none overlap. A
   stacked-column timeline sits underneath.
5. **Mosaic.** Discrete stacked columns per period (no smoothing), a nested
   treemap of harness → model sized by the metric, then token composition.
6. **Calendar.** Daily windows render a Monday-first calendar: each day fills
   from the bottom by its share of the busiest day, split by harness. Clicking a
   day selects it; shift-click or dragging extends the range. Everything
   below (headline, harness split, top models) is scoped to the selection, or
   to the whole window when nothing is selected. The 24h window renders an
   hour grid with the same selection rules. Changing the window clears the
   selection, including a round trip back to the same range. A bucket that
   falls outside the window's periods (a zone edge) gets its own cell, so the
   whole-window figure always equals selecting every visible cell.
7. **Classic.** The existing summary card, chart, activity disclosure, totals
   tiles, and breakdown, unchanged.
8. **Shared.** Coverage notes stay above every layout. History sources and
   Model prices render below every layout; Model prices is collapsed by
   default behind a chevron. The breakdown table stays
   reachable in every non-classic layout behind a `Breakdown table`
   disclosure.

## Honesty rules (every layout)

1. The headline carries the `Partial total` badge whenever the classic card
   would.
2. In cost mode the headline caption says `API estimate` and the window's
   `costSourceLabel`.
3. An unpriced model is labelled `unpriced`, and a total whose cost source is
   unpriced carries a `~` marker.
4. Reasoning is never added to processed tokens.
5. Every chart has an accessible name. Scrubbable charts are keyboard
   reachable.

## Tests

- `src/usageReport.test.ts`: `layout` round-trips and falls back to `ledger`
  on garbage without losing metric and window; model reports carry token
  kinds and dense per-period series aligned with `report.periods`.
- `src/usageGeometry.test.ts`: straight paths are finite and never smoothed;
  stacked columns sum to the period total; squarify areas are proportional
  and tile the rectangle; flow links conserve node totals in both modes;
  calendar weeks are Monday-first and dense; the calendar axis gives
  out-of-window buckets their own cells and the whole-window total equals the
  sum of visible cells; range scoping equals `buildUsageReport` over the
  filtered buckets; evenly spaced axis ticks.
- `src/components/UsageScreen.test.tsx` (jsdom): every layout renders the
  honesty strings from the summary; the layout switch persists and does not
  refetch; a stored unknown layout falls back to Ledger; Strips scrubs with
  arrow keys; Flow shows the cost-mode note and names the model in
  model → kind band titles; Calendar scopes its headline to a clicked day, a
  shift-click range, and a pointer drag, clears on a 30d → 7d → 30d round
  trip, and counts an out-of-window bucket in a visible cell. Existing classic cases keep
  passing with `layout: "classic"`.
- `bun run build` and `bun run test` green.
