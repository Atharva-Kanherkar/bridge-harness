# feat-context-lens-8-context-breakdown-ui — Test Contract

Implements [issue #246](https://github.com/Atharva-Kanherkar/bridge-harness/issues/246),
Context Lens 8/8: build the context breakdown UI. Parent #238. Consumes the
wire surface shipped by Context Lens 7/8 (#245, PR #308):
`sessions/get_context_breakdown` + `sessions/get_context_breakdown_digest`.

This contract was locked before implementation began. Design approved by the
user via an interactive HTML mock (Graphite & Paper tokens, liquid-glass
floating panel, stacked bar, ranked legend, drill-down rows).

## Functional Behavior

1. **Entry point.** The UsageWidget's context chip (`Ctx …`) opens a
   floating breakdown panel anchored to the titlebar actions. The panel is a
   genuinely floating layer and may use blur (`u-glass-liquid`); resting
   surfaces stay opaque.
2. **Pressure consistency.** The panel derives its pressure label,
   explanation, and level from the same `contextPressure()` used by
   UsageWidget (`src/usage.ts`), fed by
   `digest.conversation.contextPressure * 100`. Levels: unknown / healthy
   (<60%) / elevated (≥60%) / high (≥75%) / critical (≥90%). The panel never
   invents its own thresholds.
3. **Stacked composition bar.** One horizontal track represents
   `conversation.contextWindowTokens`. Available segments render in a fixed
   achromatic ramp ordered largest-first; every `unavailable` segment renders
   as one hatched region sized to the occupied-but-unattributed remainder;
   remaining headroom renders as empty track. A tick marks the last
   compaction boundary position when `compactionDelta` exists.
4. **No fabrication of hidden provider material.** Unavailable segments show
   their wire `reason`, render with dashed/hatched treatment, and contribute
   the unattributed remainder — never zero, never an exact number presented
   as measured. A footnote states that unavailable material still consumes
   the window.
5. **Ranked drill-down.** Rows list segments ranked by size (largest first),
   unavailable last, stable within equal sizes. Rows expand to reveal
   `names[]` items when present. Each row shows: label, token count (mono,
   tabular), % of window, and its explicit source state badge
   (`reported` / `measured` / `estimated` / `unavailable`). Estimated and
   unavailable badges are visually dashed.
6. **Source labels.** All four wire states render verbatim (capitalized).
   Segments with `state === "unavailable"` must include a non-empty reason;
   the UI renders the reason instead of a size.
7. **Compaction delta.** When `compactionDelta` exists, a delta strip shows
   `growthTokens` signed, the current estimate, and the delta reason/source
   agent when present. Absent delta renders nothing (no zeros).
8. **Worker sessions.** The panel keys everything off the focused session id
   passed down from App; a digest scoped to a worker session renders with
   that session's identity (id chip) through the same code path as an
   orchestrator session.
9. **Prompt Studio deep link.** Prompt-compilation segments
   (`prompt-stable`, `prompt-variable`) are marked editable and expose an
   "Edit in Prompt Studio" action invoking the provided callback; App routes
   to Settings → Prompt Studio (mirroring BypassBadge's pattern).
10. **Polling.** While the panel is open, a serial poll (via
    `startSerialPoll`) checks `bridgeApi.contextBreakdownDigest(sessionId)`
    on a fixed interval; only when the digest token changes is
    `bridgeApi.contextBreakdown(sessionId)` fetched (digest-gated
    reconciliation). Rules:
    - serial: next check scheduled only after the previous settles;
    - bounded: failures are swallowed but never build a backlog; after the
      fetcher reports the method missing/unavailable, polling stops;
    - session-scoped: changing `sessionId` tears down the old poll loop and
      state rather than mixing digests across sessions;
    - closed panel ⇒ no polling at all.
11. **Browser (non-Tauri) mode.** Uses the existing mock bindings, so the
    panel renders demo data in `bun run dev`.

## Unit Tests

`src/contextBreakdown.test.tsx`:

- `segments_rank_available_by_size_then_unavailable_last` — ordering rule §5.
- `unavailable_remainder_is_occupied_minus_known_and_never_negative` — §4.
- `free_headroom_uses_window_minus_occupied` — §3.
- `format_tokens_groups_thousands_and_compacts` — mono formatting.
- `delta_summary_signs_growth_and_carries_reason` — §7.
- `class_meta_labels_every_known_segment_class` — every segment class emitted
  by bridge-core (`conversation`, `prompt-stable`, `prompt-variable`,
  `providerBaseInstructions`, `toolSchemas`, `mcpDynamicTools`,
  `skillsPlugins`, `agentDefinitions`) resolves to a human label and group;
  unknown classes fall back to their raw id.
- `poll_stops_when_session_changes` — old session's in-flight/digest work
  cannot leak into the new session (§10).
- `poll_fetches_only_on_digest_change` — identical digest ⇒ no second full
  fetch (§10).
- `poll_gives_up_cleanly_when_method_is_missing` — stop flag set, no error
  surfaces (§10 bounded).

`src/components/ContextBreakdown.test.tsx` (jsdom):

- `renders_all_four_source_labels_from_the_wire` — §6.
- `unavailable_segments_render_reason_not_zero` — §4/§6.
- `rows_are_ranked_largest_first_with_unavailable_last` — §5.
- `compaction_delta_renders_signed_growth_and_reason` — §7; absent delta ⇒
  no delta strip.
- `pressure_uses_shared_context_pressure_semantics` — high-percent digest
  produces the shared "High pressure" copy (§2).
- `editable_prompt_segments_offer_prompt_studio_deep_link` — §9; callback
  receives the segment class.
- `window_totals_state_occupancy_and_headroom` — §3 math rendered.

## Integration / Functional Tests

- UsageWidget test additions: the Ctx control exposes the breakdown toggle;
  panel mount is gated behind it (no breakdown markup when closed).
- Existing UsageWidget tests remain green unchanged (pressure semantics
  untouched).

## Smoke Tests

- `bun run build` passes from the checkout root (tsc strict + vite).
- `bun run test` passes (vitest + cargo workspace untouched by this slice).

## E2E Tests

N/A — this slice adds no new harness flows; jsdom component tests cover the
panel contract end to end at the UI boundary.

## Manual / cURL Tests

A reviewer can verify visually:

```bash
bun install && bun run dev   # browser mode serves mock breakdown data
```

Open the usage widget (top-right gauge), click the `Ctx` control: the
liquid-glass panel renders the stacked bar, ranked segments with source
badges, unavailable hatch with reasons, and the Prompt Studio link on prompt
segments. In the desktop app (`bun run tauri dev`) the same flow exercises
the live wire methods.
