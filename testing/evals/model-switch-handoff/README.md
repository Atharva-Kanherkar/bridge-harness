# Model-switch handoff evals

What a model taking over a chat is handed, and what it can read back.

## Offline (in the suite, no model)

`cargo test -p bridge-core --lib handoff_eval -- --nocapture`

Seeds `cases.json` through the writers the adapters use (Claude, Codex and
OpenCode tool payloads, worker results), builds the handoff for a 32k, 200k
and 1.05M window, and scores each run: labeled files and links in the facts
block, the latest ask in `goal:`, the newest tool run in a `tools:` line, the
early fact reachable (in the handoff, or read back with `chats_history` from
the `before` it names), no planted credential in any handoff or page, and the
handoff within budget. Early facts sit in a middle message or in tool output,
since the first ask already rides in `started with:`.

## Live (the real app)

`CARGO_TARGET_DIR=<target> node scripts/e2e-model-switch.mjs` runs the branch-built `bridged` on a scratch data
directory with the real Claude and Codex logins and drives it with
`bridge exec --method`, as the desktop app does. See the script header for the
steps and what each asserts.
