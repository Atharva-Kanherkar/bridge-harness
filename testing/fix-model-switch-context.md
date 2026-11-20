# Model switches keep the conversation

Locked before implementation. Covers the first slice of the model-switch
context issue: switch summaries never become a compaction boundary, a failed
summary never builds a reconstructed one, and a chat that returns to an app it
already ran on resumes that app's own thread.

## Functional behavior and tests

### C1. A switch summary is a checkpoint, never a boundary

- A background (`before_downgrade`, model-switch) checkpoint request answered
  with a valid object appends exactly one `checkpoint` entry. It appends no
  `compaction` entry and no `branch.summary`, and it does not move
  `session_heads.active_entry_id`.
- The switch-summary outcome still reads `Summarised` for that request.
- A foreground request (manual `/compact` fallback, phase boundary, suspend)
  still writes `checkpoint` + `compaction` + `branch.summary` as before.
- Test: `compaction_controller` unit tests for both modes, and the existing
  `switch_summary` / `sessions` switch tests updated to assert no boundary.

### C2. A failed switch summary builds no wall

- When the switch summary fails (no JSON, timeout, provider exit, stopped),
  the forest gets `compaction.failed` and nothing else: no reconstructed
  `checkpoint`, no `compaction`.
- Reconstruction stays available to its other callers (dead-process recovery
  in `live_turn`) and still labels itself `reconstructed`.
- Test: a failed background request leaves no `checkpoint`/`compaction` entry.

### C3. The handoff keeps pre-switch history

- With a switch checkpoint on the branch and no compaction, the cold-start
  restoration text has the checkpoint summary (plus its decisions and open
  work) as the header, and its tail still walks entries from before the switch,
  up to the existing budget.
- The header's own checkpoint entry is not repeated in the tail.
- Test: a branch of older messages, then a switch checkpoint; the restoration
  text contains an older message and the summary appears once.

### C4. Each app's thread is remembered per chat and account

- When a switch leaves a provider thread (harness change, or an account change
  that does not share a continuation key), Bridge records the harness, account,
  continuation key, thread id and the last forest sequence the thread saw.
- A thread is not recorded for an unstarted aside (`native_fork` head), or
  when the previous account no longer exists.
- Test: `provider_threads` store round trip; `commit_chat_model_change` writes
  the row on a harness change and on a cross-home account change.

### C5. Switching back resumes that thread

- Switching to an app and account with a recorded thread, when the adapter
  supports native resume, plans a native continuation: no outgoing summary is
  requested, `sessions.provider_session_id` becomes the recorded thread id,
  the head goes to `native` with that id, and the switch notice says the chat
  goes back to its earlier session and how many messages are added.
- A different account on the same app (different continuation key) does not
  match the row. An adapter without native resume does not match.
- Same-harness model changes keep today's behavior (same thread).
- Test: plan + commit for A -> B -> A (harness change) and for an account
  flip on one harness; a mismatched key falls back to the fresh path.

### C6. What happened while away is added once

- The first native start of a returned thread carries a
  `conversation_since_you_left` section in the session-context frame, listing
  the user/assistant/worker/checkpoint entries after the recorded sequence,
  oldest first, within the restoration budget of the returning model.
- The section is owed once: after the native start it is not built again.
- If the native resume fails, the row is forgotten and the ordinary
  `checkpoint_restored` projection carries the history.
- An empty delta (nothing happened on the other app) adds no section.
- Test: `provider_threads` delta rendering and settlement; session-context
  frame includes the section only when given one.

### C7. The prompt ledger records the handoff it sent

- A `checkpoint_restored` start records the compiled prompt that includes the
  `restoration_context` section, so `prompt_compilations.variable_bytes` is
  the real size. The stable prefix hash is unchanged, so the hot-process
  compatibility check behaves as before.
- Test: ledger row for a checkpoint-restored start lists a
  `restoration_context` section.

### C8. Docs

- `docs/compaction-and-resume.md` describes switch summaries as checkpoints,
  the return-to-thread rule, and switching vs delegating.

## Out of scope

- A model-free handoff header, carrying a thread to another account, a chat
  reading its own history, tool facts in the handoff.
- Harness-native compaction and its card; budget constants; idle reaper.

## Verification

- Focused `cargo test -p bridge-core` for `compaction_controller`,
  `switch_summary`, `restoration`, `provider_threads`, `sessions`,
  `session_context`, `live_turn` before the full suite.
- `bun run build` and `bun run test` green before the PR.
