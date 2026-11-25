# One memory model contract

Issue #1146, part of epic #1144. Locked before implementation.

## Functional behavior and tests

- `memory_model::resolve` answers one harness and model for every memory job,
  trying in order: the user's pin, the application new-chat default
  (`ChatDefaults.model`, account ignored), then what a new chat starts on when
  that is unset (`sessions::resolve_orchestrator_selection`, reused, not
  restated). The first choice whose harness passes
  `briefing_policy::adapter_may_brief` wins and reports its `source`
  (`user`, `chat_default`, `standard`). Each failing choice is listed in
  `skipped` with its source and the adapter's reason.
  Test: pin wins over default; no pin uses the default; a Codex default is
  skipped with its reason and the standard choice answers; nothing eligible
  resolves to none with every skip listed.
- `memory_model::set` takes both a harness and a model, or neither (clears the
  pin). One without the other is `Invalid`. A harness that cannot run tool-free
  is `Invalid` with the adapter's reason, and the stored pin is unchanged.
  Test all three.
- Migration 98 creates `memory_model_settings` and seeds the pin from the
  legacy extraction pin, else the legacy consolidation pin, else nothing.
  Clearing the pin later is not undone by the seed. Test both legacy sources
  and the clear.
- Extraction no longer reads the chat's own harness. `enqueue_after_turn`
  queues for any visible chat once a memory model resolves, including Codex and
  OpenCode chats, and queues nothing when none resolves. `claim_due` runs on
  the memory model resolved at claim time and cancels with
  `no_memory_model` when none resolves. Test: a Codex chat queues and its
  claim carries the default's harness and model; pinning Haiku makes the next
  claim carry Haiku.
- Consolidation propose mode no longer needs its own pin. `enqueue_when_full`
  and `claim_due` read the resolved memory model; with none, the claim cancels
  with `no_memory_model`. A resolved model with no model id runs the harness
  default. Test: a scope with no legacy pin claims on the resolved model.
- Automatic approval stays the deterministic gate inside an extraction run in
  `auto_apply` mode, so it runs on the memory model with no extra wiring.
- The `harness` and `model` fields on the extraction and consolidation settings
  read back the resolved memory model and are marked deprecated. Updates ignore
  them. Test: updating consolidation to propose with no harness succeeds.
- Wire: `memory/get_memory_model` returns `pinned`, `resolved` (harness, model,
  source) and `skipped`. `memory/set_memory_model` takes optional `harness` and
  `model` and returns the same shape. Protocol 1.59, additive.
  Test: camelCase wire shapes and the version pairing.
- Agents: `memory.model` in the settings tools reads the pin (null when unset)
  with the resolved choice as `effective`, and accepts `{harness, model}`, a
  bare model id, or null, like `newChat.model`. Test read, set and clear.
- Memory dialog: the "Extraction model" card becomes "Memory model": a harness
  and model picker with "Use my new-chat model", the resolved choice and its
  source ("Using claude-opus-5-5, your new-chat model"), each skipped choice
  with its reason, and the last extraction cost. Copy says it runs extraction,
  nightly cleanup and automatic approval. Test: saving a pick calls
  `setMemoryModel` once and the source label renders.
- The `api.ts` mock serves both methods, so `bun run dev` shows the card.

## Verification

- Focused: `cargo test -p bridge-core memory_`, `-p bridge-core settings_tools`,
  `-p bridge-protocol`, `bunx vitest run src/components/MemoryDialog.test.tsx
  src/api.test.ts`.
- Regenerate protocol artifacts from the worktree root and run the drift tests.
- `bun run check` and `bun run build` before the PR; CI runs the rest.
- Screenshots of the Memory model card (dark and light) in the PR body.
