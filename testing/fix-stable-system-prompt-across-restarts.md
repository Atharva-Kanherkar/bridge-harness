# fix-stable-system-prompt-across-restarts — Test Contract

Locked before implementation. Branch cut from freshly fetched `origin/main` at
`8ad5989a` (the merge of #540, slice 2 of this epic).

Implements issue #528 (`[3/6]` of epic #525, gaps **G2** and **G9**). Slices 1
(#526, telemetry) and 2 (#527, same-harness native resume) are already on main;
this slice is the one that makes their native resume worth having.

## Why

Provider caches are prefix caches, and the system prompt precedes every message.
Bridge's stable/variable split protects Bridge's own `prefix_hash`, but the
"variable" half is still inside the system block:

- `live_turn.rs` `compile_orchestrator_prompt`, `compile_session_prompt` and
  `compile_worker_prompt` each add `session_capabilities` and `memory_packet` as
  variable sections of the compiled prompt.
- That compiled string is Claude's `systemPrompt.append`
  (`sidecar/claude-agent/options.mjs`), Codex's `developerInstructions` and
  `instructions` (`codex_adapter.rs`), and OpenCode's `system`
  (`opencode_adapter.rs` `prompt_body`).
- `session_capabilities` is `credential_broker.rs` `instructions()`, which
  embeds `proxy_token` — two fresh UUIDs on **every Bridge process start**.
- `memory_packet` is rebuilt from `memory_records ORDER BY updated_at DESC` on
  every cold start, so any memory edit changes the bytes.

So a native resume after a Bridge restart hands the provider a system block that
differs by a few bytes from the one the cached prefix was written under, and the
provider re-writes the **entire conversation** to cache. Slice 2 kept the
conversation; this slice keeps its cache.

G9 is the same shape one layer down: `prompt_body` rebuilds `body["system"]`
from `instructions + application_context` on every turn, so a turn whose text
carries a registered `[secret:sec_…]` marker mutates `system` and busts that
turn's prefix on its own.

## Functional Behavior

### 1. The compiled prompt loses its volatile sections

- `compile_orchestrator_prompt`, `compile_session_prompt` and
  `compile_worker_prompt` no longer take a credential context or a memory
  packet, and no longer emit the `session_capabilities` or `memory_packet`
  variable sections.
- What stays is genuinely fixed for the launch: the stable prefix, the worker's
  `task_context`, and `restoration_context`. Restoration context is fixed per
  launch and is legitimately part of the system context; what belongs in it is
  slice 4's question, not this one.
- Consequence, and the headline acceptance: two launches of the same session
  that differ only in proxy token and memory packet compile to **byte-identical**
  instructions. The bytes handed to `systemPrompt.append` /
  `developerInstructions` / `system` are the same across a Bridge restart.
- `validate_stable_value` is untouched. Secrets and capability markers still
  cannot enter the stable region; they now cannot enter the compiled prompt at
  all, which is strictly narrower than the rule it already enforced.

### 2. The volatile half becomes a Bridge-authored turn frame

A new `session_context` module owns it:

- `session_context::build(capabilities, memory_packet)` renders

  ```
  <bridge-session-context schema="1">
  {"sections":[{"name":"session_capabilities","text":"…"},{"name":"memory_packet","text":"…"}]}
  </bridge-session-context>
  ```

  mirroring the compiler's `<bridge-variable-context>` envelope, and a SHA-256
  digest of those bytes. Section order is fixed by construction, empty sections
  are omitted, and both empty means no frame at all.
- The frame is delivered in the conversation **tail**, as trusted
  application-owned context beside the user's message — never folded into the
  user's text, and never in the system block.

### 3. When it is delivered

`BridgeCore` keeps an in-memory ledger of what each session has already been
handed: `session_id → (provider_session_id, digest)`. Its lifetime is the Bridge
process, which is exactly the lifetime of the proxy token it protects.

- A cold launch builds the frame where the memory packet is already built today
  — past the hot return, so the retrieval audit still names a packet that is
  actually delivered — and marks it **pending** for the session unless the
  ledger already records that exact `(provider_session_id, digest)` pair.
- The next turn Bridge sends on that session carries the pending frame and, on
  success, moves it to delivered.
- Therefore:
  - **Fresh start** — nothing in the ledger, new thread: delivered. ✓
  - **Bridge restart, native resume** — ledger empty because the process is new,
    and the proxy token it would have matched is dead anyway: delivered, but as
    a small tail frame instead of a whole-prefix rewrite. ✓
  - **Same-process native resume** (the slice-2 model switch) — same thread id,
    same digest: skipped, because the frame is still in the provider's history
    and still valid. ✓
  - **Hot return** — the launch does not run, so nothing is marked pending and
    the digest already delivered stands. ✓
- `/clear` drops the ledger entry for the session alongside
  `credential_broker.clear_session`: that conversation is gone, so the frame
  Bridge believes it delivered is gone with it.
- The memory packet is still built once per cold launch, not per turn. A memory
  edit still does not reach an already-hot session — that is today's behavior
  and it is not what this issue is about.

### 4. Turn context becomes typed, and every adapter carries it

`AdapterRuntime::send_turn_with_context` and `send_turn_with_images` take an
`adapters::TurnContext { session, credentials }` instead of one opaque string,
because there are now two named things to carry and one of them must not be
labelled as the other on the wire.

- **Codex** — `turn_start_params` puts each present entry in the existing
  `additionalContext` map under its own key: `bridge.session` and
  `bridge.credentials`, each `{"kind":"application","value":…}`. An absent entry
  emits no key, and `additionalContext` is omitted entirely when both are
  absent. The user's `input` text is unchanged.
- **Claude** — gains a real `send_turn_with_context`. `user_turn_frame` puts one
  text block per present context entry **before** the user's text block, then
  the image blocks. This is the "Bridge-authored user frame through the
  streaming input" the issue asks for, delivered inside the same message rather
  than as a second one, so it cannot race the turn it belongs to.
  This also fixes a live drop: Claude's `send_turn_with_images` currently
  discards `_application_context`, so an image turn carrying a registered
  `[secret:]` marker loses the capability contract entirely.
- **OpenCode** — `prompt_body`'s `system` is built from `instructions` alone and
  is byte-identical whether or not the turn carries context. Context entries
  become leading text parts in `parts`, before the user's text.
- **Cursor / Grok** — ACP has no system channel at all: instructions are already
  folded into the first user message via `pending_instructions`. They fold the
  context entries into the same preamble, so they keep receiving the capability
  contract that used to reach them inside the compiled prompt.

### 5. Where Bridge attaches it

- `deliver_prepared_input` composes the turn's `TurnContext` from the pending
  session frame and the existing per-turn `credential_broker.turn_context`,
  and records the frame delivered on success.
- `deliver_worker_objective` does the same for a worker's first turn, which is
  how the worker's objective already reaches it.
- Bridge's own internal turns — routing notices, checkpoint prompts, steer
  envelopes — are unchanged and carry no context. Routing notices and steers go
  to a session that has already taken a turn, so they cannot be the first turn
  on a thread. A checkpoint prompt can be: `/compact` typed as the very first
  input on a fresh chat sends one. That is deliberate — a checkpoint turn asks
  the model for strict JSON about the conversation and has no use for the
  capability contract — and the frame simply stays owed until the next real
  turn.

### 6. Explicitly out of scope

- What goes into `restoration_context`, its 8 KB cap, and the switch summary —
  slice 4 (#529).
- Compaction ownership and the checkpoint contract — slice 5 (#530).
- G8's warm-worker prompt re-send at `live_turn.rs` `send_turn(&instructions)` —
  slice 6 (#531). This slice makes that re-send smaller, not absent.
- Delivering a changed memory packet to an already-hot session.
- No protocol/wire-schema change, no migration, no frontend change.

### 7. Documentation

- `testing/codex-issue-81-cache-aware-prompts.md` states that variable content
  "includes … session capability references". That is corrected with a scope
  note pointing here: capability references now ride in the turn frame, and the
  compiled prompt carries neither them nor the memory packet.
- `prompt_authority.rs`'s OpenCode note says `send_turn_with_context` rebuilds
  `system` "from `instructions` plus per-turn application context". The verdicts
  do not change — nothing here proves how OpenCode layers `system` onto its
  provider's base prompt — but the description is corrected to match the code.
- `memory_packet.rs`'s module header says the packet is "compiled into the
  variable suffix at session start, restore, and worker spawn". It is corrected
  to name the turn frame, and its source-guard test's call-site counts are
  updated with a stated reason.

## Unit Tests

Rust (`src-tauri/bridge-core`):

- `session_context::tests` — the frame is deterministic for identical inputs;
  its digest changes when the capability text changes and when the packet
  changes; a missing packet omits the section rather than emitting an empty one;
  both empty yields `None`; the rendered frame contains the proxy-auth header
  name and the packet text, which is exactly why it may not be compiled.
- `session_context::tests` — the delivery ledger: pending when the session is
  unknown; not pending when the recorded `(provider_session_id, digest)` match;
  pending when the thread id differs (a fresh thread cannot hold the old frame);
  pending when the digest differs (a new process's proxy token).
- `adapters::turn_context_tests` — entries come out in delivery order, a
  whitespace-only value is absence rather than an empty claim, and
  `folded_message` (the Cursor/Grok body-only channel) keeps the user's text
  last and unedited while leaving a context-free turn byte-identical to today's.
- `live_turn::tests` — **the headline**: `compile_session_prompt` compiled twice
  is byte-identical, and neither output contains the proxy token, the
  `x-bridge-proxy-auth` header name, the `session_capabilities` section name,
  or the `memory_packet` section name. Same assertion for
  `compile_orchestrator_prompt` and `compile_worker_prompt`.
- `live_turn::tests` — the existing `default_target_stacks_match_legacy_live_bytes`
  and `session_prompt_injects_restoration_context_like_the_orchestrator` are
  updated for the new signatures; `restoration_context` still rides in the
  variable suffix and still leaves `prefix_hash` untouched.
- `codex_adapter::tests` — `turn_start_params` carries `bridge.session` and
  `bridge.credentials` independently, omits absent keys, omits
  `additionalContext` entirely for an empty context, and leaves `input[0].text`
  exactly as submitted. The existing
  `codex_receives_the_compiled_stable_prefix_before_variable_context` ordering
  assertions still hold unchanged.
- `claude_adapter::tests` — `user_turn_frame` with no context still emits
  today's exact frame (the existing
  `a_plain_text_turn_emits_exactly_todays_frame` assertion, unchanged); with
  context it emits the context blocks first, then the user text, then the
  images in order.
- `opencode_adapter::tests` — `prompt_body`'s `system` is **byte-identical**
  with and without context (the G9 acceptance), and the context text appears in
  `parts` ahead of the user's text instead.
- `prompt_compiler::tests` — unchanged and still green, including
  `stable_prefix_rejects_secrets_and_session_capabilities` and
  `variable_suffix_is_bounded_without_affecting_valid_capability_context`.
- `memory_packet::tests::pins_enter_prompts_only_through_the_packet_gate` —
  updated call-site counts, with the gate claim itself intact: one
  `for_compile`, one helper, and every launch site going through it.

## Integration / Functional Tests

- A launch → turn round trip through a recording stub runtime: the compiled
  instructions handed to `StartRequest` contain no capability material, and the
  first turn's `TurnContext.session` does. A second turn on the same runtime
  carries no session frame.
- A same-process native resume onto the same thread id with an unchanged digest
  marks nothing pending, so its first turn carries no session frame.
- A send that the provider rejects leaves the frame owed, and the retry carries
  it. Marking it delivered on the attempt rather than the success would lose
  the capability contract for the rest of the conversation.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core session_context::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core prompt_compiler::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core memory_packet::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core codex_adapter::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core claude_adapter::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core opencode_adapter::`
- `bun run check`
- `bun run build`
- `bun run test`

Rust and frontend failures are diffed against `main`; only a delta counts as a
regression. Known flakes carried from slices 1–2 are expected to recur and are
not regressions: `src/transcript/flood.perf.test.ts` (timing ratio) and
`process_ledger::tests::global_root_registration_round_trips` (shared
directory).

## E2E Tests

N/A automated — no fixture can spawn a provider process, so the "restart twice
and watch `cache_read_tokens`" half of the issue's acceptance is a manual check
below rather than a test.

## Manual / cURL Tests

On a dev build against a scratch `BRIDGE_DATA_DIR`:

1. Hold a short conversation on Claude. Quit Bridge. Reopen it and send one more
   message in the same chat. The resumed turn must read as a continuation, and:

```sql
-- the resumed turn is native (slice 2) and now also cache-warm (this slice)
SELECT restoration_mode, cache_read_tokens, cache_write_tokens, uncached_input_tokens
FROM usage_ledger WHERE session_id='<id>' ORDER BY created_at DESC LIMIT 3;
```

`cache_read_tokens` on the first turn after the restart should be close to the
conversation's history rather than near zero. Before this change it is near
zero, because the append differed by a few bytes.

2. With OpenCode, send a turn containing a registered `[secret:sec_…]` marker and
   one without. Both turns' `system` must be identical; the capability text must
   appear in the turn's parts. Codex is the same check against
   `additionalContext`.

3. Edit a memory record and start a **new** chat. The packet must still reach
   it — via the turn frame now, not the system prompt — and
   `memory_retrieval_audits` must still hold exactly one row for that launch.
