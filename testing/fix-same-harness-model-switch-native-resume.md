# fix-same-harness-model-switch-native-resume — Test Contract

Locked before implementation. Branch cut from freshly fetched `origin/main` at
`d7bebd33` (the merge of #536, slice 1 of this epic).

Implements issue #527 (`[2/6]` of epic #525, gaps **G1** and **G10**). Out of
scope: handoff size and the async switch summary (slice 4, #529), prompt
delivery (slice 3, #528).

## Why

`sessions.rs` `persist_chat_model_selection` sets `provider_session_id=NULL` in
**both** branches, and `commit_chat_model_change` then writes
`RestorationMode::Fresh`. `restoration.rs` `select_plan` can therefore never
return `Native`, so a Sonnet→Opus switch discards the whole Claude Code session
exactly like Codex→Claude does and restarts from the 8 KB projection.

The provider prompt cache is per model, so the cache miss on a model change is
unavoidable. Losing the conversation is not. Both harnesses already support it:
the Claude Agent SDK sets `resume` and `model` independently
(`sidecar/claude-agent/options.mjs:54-56`), and Codex `thread/resume` carries
`model`. The effort-only path in `api.rs` is the existing proof of the shape —
it keeps the native identity and just restarts the runtime.

## Functional Behavior

### 1. A same-harness model switch keeps the provider session

- The same-harness branch of `persist_chat_model_selection` keeps
  `provider_session_id` and the `backend_*` binding. It still clears
  `active_turn_id`/`ended_at` and returns the row to `idle`.
- `commit_chat_model_change` writes `RestorationMode::Native` /
  `ResumeEligibility::Native` for a same-harness change, leaving the stored
  thread id in place.
- The next turn's `select_plan` returns `Native`, the adapter resumes the stored
  thread under the new model, and `usage_ledger.restoration_mode` for that turn
  is `native`.
- The cross-harness branch is unchanged: it still clears the provider session,
  the backend binding, and the head, and still restores from the projection.

### 2. A same-harness switch does not summarise

- `summarise_for_switch` is skipped entirely when the harness is unchanged: the
  provider keeps the conversation, so there is nothing to hand over.
- No `compaction.requested` entry is appended by a same-harness switch, and
  therefore no `compaction.failed` from its timeout.
- A cross-harness switch still summarises exactly as today, with the same
  bounded budget and the same swallow-on-failure behavior.

### 3. The transcript tells the truth, and still shows the milestone

- `session.model_changed` is still appended for both kinds of switch — the user
  changed the model, which is a real boundary in the conversation.
- For a same-harness switch the detail text says the conversation continues on
  the same provider session, and `data.freshProviderSession` is `false`. It does
  not claim carried context, because nothing was carried; `data.carriedContext`
  is absent.
- For a cross-harness switch the text and `freshProviderSession: true` are
  unchanged.
- Because `freshProviderSession === true` is currently the *only* key the
  frontend uses to render the milestone (`AgentConversation.tsx` `ItemView`) and
  to close an activity group (`transcript/grouping.ts`), a stable
  `data.modelChanged: true` is added to every `session.model_changed` payload and
  both call sites accept it. The legacy `freshProviderSession` key is still
  accepted so entries written before this change keep rendering.

### 4. A cross-harness switch clears the stale native id (G10)

- `set_head_state` upserts `native_provider_session_id` with
  `COALESCE(excluded, existing)`, and the switch passes `None`, so
  `session_heads.native_provider_session_id` keeps the dead pre-switch thread id
  while `sessions.provider_session_id` is NULL, and the forest snapshot surfaces
  it.
- A cross-harness switch now clears that column explicitly. After one,
  `session_heads.native_provider_session_id IS NULL`.
- Every other caller of `set_head_state` keeps today's coalescing behavior: an
  explicit clear is opt-in, never the default, so a launch that does not know the
  thread id cannot wipe a good one.

### 5. Codex carries reasoning effort across a resume

- `thread_resume_params` currently sends no effort at all, so a switch that
  changes model *and* effort would resume at the thread's previous effort.
- The app-server's generated schema for codex-cli 0.153.4 has no `effort` field
  on either `ThreadStartParams` or `ThreadResumeParams`; both accept a permissive
  `config` map, and `model_reasoning_effort` is the Codex config key for it.
  Resume therefore carries effort as `config.model_reasoning_effort`.
- `thread_start_params` gains the same `config` entry so start and resume agree.
  Its existing top-level `effort` / `model_reasoning_effort` fields are kept:
  both param types are permissive, so they cost nothing and preserve any build
  that did read them.
- `thread_fork_params` is left alone — a fork is an aside, and its effort is not
  what this issue is about.

### 6. Documentation

- `testing/feat-336-model-switch-handoff.md` currently states as behavior 1 that
  a same-harness switch delivers the stored conversation as labelled restoration
  context, and lists "same-harness native restarts" as explicitly out of scope.
  Both notes are corrected to point at this contract, since #527 supersedes them.

### 7. Explicitly out of scope

- No change to `checkpoint_context`'s 8 KB / 12-line cap or to the summary's
  place on the critical path — that is #529.
- No change to prompt assembly or delivery — that is #528.
- No change to compaction triggers, gates, or the checkpoint contract — that is
  #530.
- No protocol/wire-schema change.

## Unit Tests

Rust (`src-tauri/bridge-core`):

- `sessions::tests` — `persist_chat_model_selection` on a same-harness change
  leaves `provider_session_id` and `backend_id` intact; on a cross-harness change
  it clears both. Same test asserts the guarded UPDATE still matches on the
  previous harness/model and still returns 0 when the row moved underneath.
- `sessions::tests` — after `commit_chat_model_change` for a same-harness change,
  `session_heads.restoration_mode` is `native`, `resume_eligibility` is `native`,
  and `native_provider_session_id` is unchanged.
- `sessions::tests` — after a cross-harness `commit_chat_model_change`,
  `session_heads.restoration_mode` is `fresh` and `native_provider_session_id`
  IS NULL.
- `sessions::tests` — the `session.model_changed` payload for a same-harness
  change has `modelChanged: true`, `freshProviderSession: false`, and no
  `carriedContext`; the cross-harness payload keeps `freshProviderSession: true`
  and gains `modelChanged: true`.
- `restoration::tests` — `select_plan` with a retained provider id and a native
  adapter returns `Native`, which is what the kept id buys. `set_head_state`
  still coalesces a `None` id by default and clears it when asked.
- `codex_adapter::tests` — `thread_resume_params` carries
  `config.model_reasoning_effort` when an effort is given and omits `config`
  when it is not; `thread_start_params` carries it too and keeps its existing
  fields.
- Existing `sessions::tests::lifecycle_claims_serialize_starts_against_model_switches`
  and `switch_summary_outcome_*` still pass unchanged, or are updated with a
  stated reason.

Frontend (Vitest):

- `src/transcript/grouping.test.ts` — an item carrying only `modelChanged: true`
  closes the activity group, and the legacy `freshProviderSession: true` item
  still does.
- `src/components/AgentConversation.test.tsx` — a `session.model_changed` entry
  with `freshProviderSession: false` and `modelChanged: true` still renders the
  model-changed row.

## Integration / Functional Tests

- A same-harness switch followed by a cold start selects `Native` and issues a
  resume rather than a fresh start (asserted at the plan/params level; no test
  fixture can spawn a provider process — every stub's `start()` errors by
  design, as `feat-336`'s contract already records).
- A cross-harness switch followed by a cold start still selects
  `CheckpointRestored`.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core sessions::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core restoration::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core codex_adapter::`
- `bunx vitest run src/transcript/grouping.test.ts src/components/AgentConversation.test.tsx`
- `bun run check`
- `bun run build`
- `bun run test`

Rust and frontend failures are diffed against `main`; only a delta counts as a
regression. Two known flakes from slice 1 are expected to recur and are not
regressions: `src/transcript/flood.perf.test.ts` (timing ratio) and
`process_ledger::tests::global_root_registration_round_trips` (shared directory).

## E2E Tests

N/A automated — no fixture can spawn a provider process.

## Manual / cURL Tests

On a dev build against a scratch `BRIDGE_DATA_DIR`, hold a short conversation on
Claude, switch Sonnet→Opus from the chat header, then ask a question that depends
only on the earlier turns ("what file did we decide to change?"). It must be
answerable without re-explaining, and:

```sql
-- native, not checkpoint_restored
SELECT restoration_mode, count(*) FROM usage_ledger
WHERE session_id='<id>' AND restoration_mode IS NOT NULL GROUP BY 1;

-- the switch appended no compaction request
SELECT kind, count(*) FROM session_entries
WHERE session_id='<id>' AND kind LIKE 'compaction%' GROUP BY 1;

-- the thread id survived
SELECT provider_session_id IS NOT NULL, continuation_fidelity FROM sessions WHERE id='<id>';

-- and after a cross-harness switch, the stale id is gone
SELECT native_provider_session_id FROM session_heads WHERE session_id='<id>';
```

Then confirm the transcript still shows the model-changed divider for the
same-harness switch, and that its text does not claim a fresh provider session.
