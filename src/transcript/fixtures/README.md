# Golden streams

One logical turn, four harnesses, in the shape each Rust adapter actually
publishes it. Derived from the normalizers in
`src-tauri/bridge-core/src/agent.rs` and `acp_events.rs`, and from
`testing/fixtures/builtin-adapter-events-v1.json`, which is the Rust side's own
expectation table for the same normalizers.

The turn, in every file:

1. the user's message
2. a thought
3. a command that streams output and finishes
4. a second thought, after the command
5. a file read
6. a patch
7. the assistant's reply
8. turn completed

Each file is an array of raw events exactly as `bridgeApi.onAgentEvent`
delivers them. Persisted frames carry a forest `sequence`; transient frames
(deltas, progress, turn markers) carry `sequence: 0` and `id: 0`, which is what
the backend publishes for anything the forest refuses to store.

What differs, and why, is asserted in `../golden.test.ts`:

- **claude** — tool calls arrive consecutively, one `tool_use` block per call,
  and are completed by a matching `tool_result`. Claude streams no tool output
  and reports no exit code; the file family is decided by tool name
  (`Bash` → command, `Edit`/`Write` → file change, everything else → tool) and
  the patch is synthesized by the adapter from `old_string`/`new_string`.
- **codex** — items with a `type`, streamed through `item/started`,
  `item/commandExecution/outputDelta` and `item/completed`. Reasoning gets a
  fresh item id per block, so a thought after a command is a second card rather
  than an append to the first. Reports an exit code.
- **cursor** — ACP. Every call is `tool.started`/`tool.completed` with the
  category on `data.kind` and the payload under `data.update`; output lives in
  the update's content blocks, and paths in its `locations`. No exit code — the
  protocol has no field for one. A thought streams as `agent_thought_chunk`
  updates and the protocol never says it finished, so `acp_events.rs` closes the
  run itself: the `reasoning.completed` frames here are the ones it assembles,
  carrying the run's text under the id its deltas carried. They are persisted,
  which is what gives a replayed Cursor turn its thoughts back.
- **opencode** — parts with a nested `state`. A running tool re-sends the whole
  part, so progress arrives as a repeated `command.started` rather than as a
  delta. Arguments live under `state.input`, output under `state.output`, and
  the exit code under `state.metadata.exit`.

## The flood

`codexFlood.ts` is the same turn scaled up: one hundred steps, with a fresh
reasoning item between every tool item, which is what the item-prefixed
emitter really publishes on a long turn. It is generated rather than written
out — four hundred hand-written frames would be unreviewable, and what is
under test is the shape, not any particular command. The recipe is the
fixture: exactly 62 commands, 30 reads and 8 patches, interleaved, wrapped in
a user message, an opening thought, a closing thought and the reply.
`src/components/AgentConversation.flood.test.tsx` and
`src/transcript/flood.perf.test.ts` read it.
