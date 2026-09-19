# feat/memory-extract

The first LLM writer, and it can only propose. A post-turn extraction run on
the chat's own harness and model, or on a helper the user pinned — never
selected by the learning router — produces proposed records for the review
queue. The queue ships in this slice because its first content ships in this
slice.

Propose is the default. With extraction off by default the ledger held only
what users typed into `/pin`, which almost nobody did, and testers reported
memory as present but never used. Nothing a run proposes activates without
the user.

## Schema 35

- `memory_records` gains nullable `confidence_bps` and `rationale`, arriving
  with their first honest producer. Explicit saves keep them NULL and render
  as unknown, never a fake certainty.
- `memory_extraction_settings`: one row per scope; `mode` is `propose`
  (default) or `remember` (nothing automatic); plus an optional pinned
  `harness` and `model`, saved together or not at all. Unpinned, a run
  resolves to the finished chat's own harness and model; a chat on a harness
  that cannot run tool-free is not enqueued at all rather than queued and
  cancelled after every turn. `auto_apply` is refused by the store until a
  replay bench exists — the same honesty as the evaluator ceilings.
- `memory_extraction_runs`: every run is a written row — queued, leased,
  heartbeat, and settled `completed | failed | cancelled` with adapter, model,
  prompt digest, observed tokens and spend, proposal count, and detail. Spend
  is observed, never hardcoded.

## The run

- Enqueued after a completed turn of a visible chat when the scope's mode is
  `propose`. Hidden kinds (briefing, extraction) never enqueue; one open run
  per session at a time. Turning `propose` off stops new runs immediately.
- The digest is bounded and typed: recent conversational entries by id plus
  the scope's active pin bodies, character-capped. Never a raw transcript.
- The executor is the resolved profile through a hidden session
  (`kind = 'extraction'`, no workspace, empty briefing scope so every tool
  call is denied), one turn, wall-clock bounded. The learning router is never
  consulted, and an extraction run writes no router decision.
- The model answers one fenced `bridge-memory-proposals` JSON array of
  `{body, kind, confidenceBps, rationale}`. Unknown fields — a `status`, a
  `provenance`, a `scopeKey` — make the proposal invalid rather than
  honored. At most 10 proposals per run survive.
- The deterministic gate re-runs everything an explicit save enforces: body
  cap, secret interception, kind vocabulary, plus confidence clamped to
  0..=10000, rationale bounded, and digest dedupe against every non-deleted
  record in scope. Survivors land `proposed` / `model_proposal`. Nothing a
  model returns can reach `active`.
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
- The Memory dialog grows an About me / Review queue tab pair. Queue rows show
  body, kind, confidence, and rationale, with keyboard-operable Approve and
  Reject. The mode control says Remember / Propose / Auto-apply, with
  Auto-apply visibly disabled until the bench exists. The words live on the
  Memory surface; the router dialog never grows an extraction toggle.

## Out of scope

Auto-apply behavior, router-routed extraction, the evaluator executor,
consolidation, embeddings, prompt injection of any record, supersede on the
wire (the UI catch-up slice owns it), workspace-scoped settings beyond
account:local.
