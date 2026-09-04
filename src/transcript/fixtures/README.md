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
  protocol has no field for one.
- **opencode** — parts with a nested `state`. A running tool re-sends the whole
  part, so progress arrives as a repeated `command.started` rather than as a
  delta. Arguments live under `state.input`, output under `state.output`, and
  the exit code under `state.metadata.exit`.
