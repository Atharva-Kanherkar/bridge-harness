# feat-context-lens-7-context-breakdown — Test Contract

Implements [issue #245](https://github.com/Atharva-Kanherkar/bridge-harness/issues/245),
Context Lens 7/8: add context breakdown API and digest. Parent #238. Depends on
Context Lens 5/8 (#286, prompt accounting) and 6/8 (#287, adapter context
inventory) — both merged at `f2ec252a` / `e21601f9`.

This contract was locked before implementation began.

## Functional Behavior

New typed method `sessions/get_context_breakdown` with params `{ sessionId }`,
returning a bounded, source-labelled breakdown for one session. Wire plumbing
mirrors `sessions/get_session_forest_digest` end to end:

| Layer | File |
| --- | --- |
| Payload types | `src-tauri/bridge-protocol/src/messages/sessions.rs` |
| Method table | `src-tauri/bridge-protocol/src/methods.rs` |
| Message registry | `src-tauri/bridge-protocol/src/messages/mod.rs` |
| Dispatch | `src-tauri/bridged/src/dispatch.rs` |
| Core API | `src-tauri/bridge-core/src/api.rs` |
| Tauri command | `src-tauri/src/lib.rs` |
| Generated TS artifact | `src/protocol/generated/protocol.ts` (via `bridge-protocol/src/bin/generate-protocol-artifacts.rs`) |
| Generated JSON schemas | `docs/protocol/schemas/*.json` |
| Browser mock binding | `src/api.ts` |

Behavior:

1. **Conversation state** is computed from `SessionForest::active_branch`
   followed by `ContextProjector::project(&branch, window)`. The window follows
   the `restoration.rs` precedent (128_000). The uncompacted
   `compaction_controller::active_token_estimate` semantics must NOT be used
   anywhere in this feature.
2. **Three sources merge into segments**, each preserving its source label and
   availability state, reusing CL6's `ContextSegmentObservation` semantics
   (`reported` / `measured` / `estimated` / `unavailable`):
   - conversation projection (CL core `context.rs`),
   - latest prompt accounting record (`store::latest_prompt_compilation`,
     CL5/#286),
   - live adapter context inventory (`context_inventory`, CL6/#287).
3. **No fabrication**: a missing compilation or absent adapter inventory yields
   an explicit `unavailable` segment with a reason string — never zeros passed
   off as data, never attribution to a source that did not report. Totals sum
   only available observations and carry their own availability state.
4. **Compaction delta**: retrieve the previous valid compaction snapshot
   (checkpoint records already carry `tokens_before`,
   `first_retained_entry_id`, `reason`) and report delta against the current
   state; `null` when no prior snapshot exists.
5. **Cap and ordering**: segment count is capped by
   `MAX_CONTEXT_BREAKDOWN_SEGMENTS` (64, declared beside
   `MAX_REPLAY_EVENT_LIMIT`); ordering is deterministic (sorted by origin then
   segment class) so two calls on unchanged state return identical payloads.
6. **Breakdown digest**: a lightweight opaque token (same contract as
   `SessionForestDigestResult.digest`) that changes when any of these change,
   and only when these change: prompt compilations, prompt-section revisions
   (`prompt_section_revisions` store), adapter context observations, and
   active-branch changes.
7. **Coverage**: works for orchestrator, worker (`parent_session_id` set), and
   direct sessions through one code path keyed by `session_id`.

## Unit Tests (bridge-core)

- `context_breakdown_computes_for_orchestrator_worker_and_direct_sessions` —
  same code path covers `Session.kind` variants; worker sessions resolve
  through their parent chain where required.
- `context_breakdown_orders_segments_stably` — repeated computation returns
  byte-identical ordering without insertion-order dependence.
- `context_breakdown_caps_segments_at_limit` — > 64 candidate segments
  truncate deterministically.
- `context_breakdown_marks_unavailable_sources_without_fabrication` — no
  recorded compilation and no adapter inventory produce `unavailable` segments
  with reasons; totals exclude them entirely.
- `context_breakdown_reports_compaction_delta_from_previous_snapshot` — with a
  checkpoint present, delta uses stored `tokens_before`; without one, delta is
  null and nothing is fabricated.
- `context_breakdown_digest_is_stable_and_tracks_all_four_inputs` — digest
  unchanged across no-op recomputes; changes exactly once per: new prompt
  compilation, prompt-section revision, adapter observation, active-branch
  switch.
- `get_context_breakdown_errors_for_unknown_session` — unknown id errors like
  other sessions methods.

## Typed Protocol Tests (bridge-protocol)

- Params/result serde round-trip in camelCase; params reject unknown fields
  (`deny_unknown_fields`, matching sibling params types).
- Registration tuples in `methods.rs` + `messages/mod.rs` compile and map the
  new method name.

## Integration Tests

- **Mirror**: `assert_mirrors::<wire::GetContextBreakdownParams>` and
  `<wire::ContextBreakdownResult>` in `protocol_mirror.rs`.
- **Dispatch**: `MethodName::GetContextBreakdown` decodes params and replies
  from `api::get_context_breakdown`.
- **Tauri**: command exposed in `lib.rs` invoke handler; delegates to core API.
- **Generated artifacts**: running
  `bridge-protocol/src/bin/generate-protocol-artifacts.rs` regenerates
  `protocol.ts` and `docs/protocol/schemas/*.json` with zero diff against the
  committed files.
- **Browser mock**: non-Tauri `api.ts` path returns a stub satisfying the
  generated TS interface; Vitest asserts shape and that it does not throw.

## Smoke Tests

- `bun run build` green (tsc -b && vite build).
- `bun run test` green (vitest run + cargo test) including all pre-existing
  forest-digest tests.

## E2E Tests

N/A — desktop app; covered by the smoke suite plus Tauri command wiring above.

## Manual Tests

From the Bridge app (dev build):

1. Open an orchestrator chat with a live worker → call
   `sessions/get_context_breakdown` via the devtools protocol console →
   segments include projection + compilation + adapter-inventory entries with
   distinct source labels.
2. Open a fresh direct session before any turn → compilation and adapter
   segments show `unavailable` reasons; totals still render.
3. Poll twice with no activity → identical `digest`; send one message →
   digest changes.

## Known Limitations (documented, not blockers)

- No production source for `context_window_tokens` exists; the 128_000 default
  follows `restoration.rs`. The window value is echoed in the result so clients
  see the basis.
- Adapter inventory is in-process state; it participates in breakdown responses
  and digests on the live instance but not in cross-restart persistence.
