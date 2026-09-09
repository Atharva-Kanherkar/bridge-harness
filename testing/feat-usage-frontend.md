# Contract: token and cost usage — frontend

Branch `feat/usage-frontend`. Frontend half of #584, over the wire surface
shipped by `feat/usage-tracking-backend`: `usage/summary`,
`usage/list_price_overrides`, `usage/set_price_override`,
`usage/clear_price_override`, `usage/refresh_rates`,
`usage/list_history_sources`, `usage/scan_history`. Locked before
implementation. T3 Code's `UsagePage` is the design reference for structure
and honesty rules; nothing is copied verbatim, and the surface is rendered in
Bridge's Graphite & Paper chrome with Tailwind v4 utilities only.

## Surface

1. **Entry point.** A `Usage` row in the sidebar's main navigation opens a
   canvas view beside the sidebar (`AppView` gains `"usage"`), like Memory
   and Marketplace. The chrome title reads `Usage`. Back navigation returns
   to the previous place.
2. **Controls.** A metric segmented control (`Cost | Tokens`) and a window
   segmented control (`24h | 7d | 30d | 90d`), a `Include history` toggle,
   and a refresh button. Metric, window, and the history toggle persist in
   `localStorage` under one versioned key; a malformed blob falls back to the
   defaults (`cost`, `30`, history on) rather than throwing.
3. **Window computation.** Daily windows use calendar arithmetic in the
   user's IANA time zone (`Intl.DateTimeFormat().resolvedOptions().timeZone`,
   degrading to `UTC`), inclusive on both ends, so a window never shifts a
   calendar day around DST. The 24h window requests `resolution: "hour"`
   with minute-floored `sinceTime`/`untilTime` exactly 24 hours apart and
   `sinceDay`/`untilDay` covering the local days those instants touch.
4. **Summary rail.** One hero number (total cost or total processed tokens),
   a subtitle with the session count and, in cost mode, `API estimate`.
   Below it one row per harness with activity, showing the primary value and
   a secondary line that cross-references the other metric
   (`42.0% of cost · 19.9M tokens`). The harness set is the set with any
   tokens or cost regardless of the displayed metric, so toggling the metric
   never adds or removes rows or series.
5. **Chart.** A hand-rolled SVG, one layered area series per harness, dense
   over every period in the window (gap periods render as zero, not as
   missing points). The y-scale peaks at the largest single harness-period
   with nice ticks; the x-axis prints first, middle, and last labels. Series
   render in a fixed achromatic ramp ordered by total, heavier series painted
   first, all fills before all strokes. Hovering snaps to the nearest period
   and shows a tooltip listing every active harness (zeros included) plus a
   total. No charting library.
6. **Totals tiles.** Processed tokens, cached input, uncached input, output,
   reasoning (labelled as part of output, never added), cache savings (USD).
7. **Breakdown.** A `Model | Time` toggle (local state, not persisted). The
   model table lists `Model · Harness | Cost | Share | Tokens`; the time
   table lists one row per period newest-first with one cost column per
   active harness plus total and tokens. Empty windows say
   `No activity in this window.`
8. **Sources and pricing.** A section lists every history source with its
   coverage state, records imported/skipped, last successful scan, and any
   coverage reason or error; unsupported sources show their reason. A
   With `Include history` enabled, opening Usage or refreshing incrementally
   scans local history before declaring its total complete. `Scan history`
   enables history and performs the same scan. Each API call remains bounded;
   partial sources are resumed until complete, with progress visible. Failed
   or non-advancing sources stop with a partial-total warning, not an endless
   retry. A pricing line shows the rate snapshot date, known model count,
   override count, and a `Refresh rates` action calling `usage/refresh_rates`
   and then refetching.
9. **Model prices.** A table over models seen in the window plus existing
   overrides, in USD per million tokens (input, output, cache read, cache
   write). Saving converts to integer micro-USD and calls
   `usage/set_price_override`; `Reset` calls `usage/clear_price_override`.
   Both refetch the summary so history reprices immediately.

## Honesty rules

1. **Never invent a number.** Unpriced buckets count tokens and contribute
   zero cost; when any bucket in the window is `unpriced`, the summary shows
   how many records were unpriced and the cost carries an `estimate` note.
   The cost hero says `API estimate`; the description says plainly that it
   is not money spent.
2. **Provenance is visible.** The weakest cost source in the window is
   displayed (`Provider reported`, `Model priced`, `Partly unpriced`).
3. **Reasoning is a breakdown, not an addend.** Processed tokens =
   uncached input + cache read + cache write + output. Reasoning is shown
   separately and never summed into totals.
4. **Deduplication is reported, not hidden.** `duplicatesDropped > 0` renders
   a note that live rows were counted once against imported transcripts.
5. **Partial coverage is stated.** Sources whose coverage is not `complete`
   render their state verbatim next to the source, and a `partial` or
   `stale` source is called out above the chart. This includes discovered
   sources that have never been indexed. Loading or failed imports label
   the hero `Partial total`; disabling history explicitly says `Bridge
   sessions only`.

## Data fetch

- One `usage/summary` call per window/metric-independent key; toggling the
  metric never refetches. Refresh, scan, rate refresh, and override edits
  refetch. Completing a history scan refetches the current window. No polling
  and no focus refetch. Window, metric, and rate changes do not restart history
  scans. Unmounting or disabling history stops scheduling further batches;
  late summary responses cannot replace a newer selection.
- Outside Tauri the mock host returns a deterministic multi-harness window
  so `bun run dev` renders the full page.

## Tests

- `src/usageReport.test.ts`: window computation (day and hour, DST-safe day
  math), formatting (`formatTokens` three significant figures, USD 2dp,
  `<$0.01` floor), aggregation by harness/model/period with dense periods,
  `niceScale`, monotone curve path is finite and closed, preference
  round-trip and fallback.
- `src/components/UsageScreen.test.tsx` (jsdom): renders summary from the
  mock api, toggles metric without refetching, switches window and refetches
  with the new params, shows the unpriced note and the source coverage
  state, scan action refetches, empty window copy.
- `bun run build` and `bun run test` green.

## Regression: missing device history

Locked before this correction on `fix/usage-summary-honesty`:

- The live database had zero history sources and observations despite the
  default history toggle being enabled. T3's cached, deduplicated Aug 11-Sep 9
  records in Asia/Kolkata priced to $5,455.37 Claude + $739.19 Codex.
- Component tests must prove automatic import, more than one bounded batch,
  retrying only partial sources, progress with an unchanged imported count
  but an advancing cursor, stopping on a stuck cursor or failure, explicit
  history-off scope, cancellation, and late-response protection.
- Replay real local history into an isolated store and report the before/after
  summary using the production importer and aggregator. Do not rewrite the
  live ledger or change rates to force agreement with T3.
- Desktop smoke: open Usage with history enabled, observe progress to
  completion, refresh, switch windows, and turn history off. The total must
  state its scope, and pending/failed history must never look complete.
