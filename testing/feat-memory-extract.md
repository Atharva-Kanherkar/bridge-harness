# feat/memory-extract

The first LLM writer always goes through one typed proposal gate. A post-turn
extraction run on the chat's own harness and model, or on a helper the user
pinned — never selected by the learning router — either leaves a candidate in
the review queue or, in explicit `auto_apply` mode, promotes a conservative
subset through the existing proposal lifecycle.

Propose is the default, so existing installations retain a review-first
workflow. Automatic promotion is opt-in, and candidates that pass validation
and fit the scope budget but remain uncertain fall back to review rather than
being activated. Invalid, unsafe, duplicate, and over-budget output is refused.

## Schema 35

- `memory_records` gains nullable `confidence_bps` and `rationale`, arriving
  with their first honest producer. Explicit saves keep them NULL and render
  as unknown, never a fake certainty.
- `memory_extraction_settings`: one row per scope; `mode` is `propose`
  (review-first default), `remember` (manual only), or `auto_apply`; plus an optional pinned
  `harness` and `model`, saved together or not at all. Unpinned, a run
  resolves to the finished chat's own harness and model; a chat on a harness
  that cannot run tool-free is not enqueued at all rather than queued and
  cancelled after every turn.
- `memory_extraction_runs`: every run is a written row — queued with the mode
  that authorized it, leased, heartbeat, and settled `completed | failed |
  cancelled` with adapter, model, prompt digest, observed tokens and spend,
  proposal count, and detail. Spend is observed, never hardcoded. Pre-snapshot
  rows migrate to `propose`, so an upgrade grants no automatic authority.

## The run

- Enqueued after a completed turn of a visible chat when the scope's mode is
  `propose` or `auto_apply`. Hidden kinds (briefing, extraction) never enqueue;
  one open run per session at a time. `remember` stops new runs immediately.
- The digest is bounded and typed: recent conversational entries by id plus
  the scope's active pin bodies, character-capped. Never a raw transcript.
- The executor is the resolved profile through a hidden session
  (`kind = 'extraction'`, no workspace, empty briefing scope so every tool
  call is denied), one turn, wall-clock bounded. The learning router is never
  consulted, and an extraction run writes no router decision.
- The model answers one fenced `bridge-memory-proposals` JSON array of
  `{body, kind, confidenceBps, rationale}`. The rationale begins with the id of
  the visible user message that directly supports the claim. Unknown fields — a `status`, a
  `provenance`, a `scopeKey` — make the proposal invalid rather than
  honored. At most 10 proposals per run survive.
- The deterministic gate re-runs everything an explicit save enforces: body
  cap, secret interception, kind vocabulary, plus confidence clamped to
  0..=10000, rationale bounded, and digest dedupe against every non-deleted
  record in scope. Every survivor that fits the scope budget first lands
  `proposed` / `model_proposal`; overflow is refused rather than evicting or
  silently dropping existing memory.
- In `auto_apply`, promotion additionally requires raw confidence in
  `9000..=10000`, a cited visible user message in the same session, lexical
  grounding and an explicit durable cue in the same unquoted clause, no
  transient or retraction markers, no likely tombstone match, and no likely
  conflict with active, proposed, or same-batch memory. Batch conflicts are
  symmetric: ordering cannot activate the first of two competing candidates.
  Eligible rows use the ordinary `approve` transition so the validity interval
  and FTS index open atomically. Any failed check leaves the validated,
  within-budget row proposed. Both the enqueue-time and current modes must be
  `auto_apply`; a setting change can only downgrade a run, never escalate it.
- The run pipeline is written against an extraction-model trait; no test
  performs a model call, and opt-out is absolute: with `remember` mode nothing
  reaches an adapter.

## The queue and the wire

- `MemoryRecord` gains optional `confidenceBps` / `rationale`. List accepts an
  optional `status` of `active` (default) or `proposed`; nothing else.
- New methods: `memory/approve_memory_record`, `memory/reject_memory_record`,
  `memory/get_extraction_settings`, `memory/update_extraction_settings`.
  Approve and reject reuse the lifecycle machine; both publish
  `memory-changed`.
- The Memory dialog has an About me / Review queue tab pair. Queue rows show
  body, kind, confidence, and rationale, with keyboard-operable Approve and
  Reject. The mode control says Manual only / Review first / Automatic and
  explains the automatic gate and review fallback. The words live on the
  Memory surface; the router dialog never grows an extraction toggle.

## Out of scope

Router-routed extraction, a learned promotion threshold, embeddings, and
workspace-scoped extraction settings beyond `account:local`.
