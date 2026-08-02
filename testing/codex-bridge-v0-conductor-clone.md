# codex/bridge-v0-conductor-clone — Test Contract

## Contract Revision — Structured Harness GUI

The agent experience is no longer allowed to render a harness TUI. This revision supersedes any earlier acceptance criterion that placed Claude or Codex inside xterm. PTYs remain valid only for the explicitly separate workspace Terminal tool.

### Normalized Harness Primitives

- Every coding harness connects through a registered adapter implementing the same lifecycle: discover capabilities, start/resume a session, send a user turn, cancel a turn, answer an approval, and stop the session.
- Adapters emit versioned normalized events. The core vocabulary includes session status, assistant/user messages and streaming deltas, reasoning summaries, plans and plan steps, tool calls and results, command executions, file changes/diffs, approval requests/resolutions, usage/context updates, errors, and artifacts.
- The core, database, and UI depend only on normalized primitives—never on Codex JSON-RPC method names, Claude event names, ANSI parsing, or harness-specific payload shapes.
- Harness-specific data is retained as optional namespaced metadata for debugging and forward compatibility. Unknown events do not crash or corrupt a session.
- Adapter capabilities declare which optional primitives are supported, allowing future harnesses to join without changes to the session core or conversation renderer.

### GUI Requirements

- The Agent tab renders a structured conversation: human and assistant messages, streaming state, collapsible reasoning, tool/command activity, plan progress, approvals with explicit actions, file-change summaries, errors, and artifacts.
- No ANSI terminal screen, raw TUI frame, or xterm component is mounted anywhere in the Agent tab.
- Composer submissions create normalized user turns and go through the active adapter.
- Tool calls and approvals remain interactive structured objects rather than copied terminal text.
- Session history reconstructs from persisted normalized events after app restart.
- A separate Terminal tab may expose a workspace shell PTY for ad-hoc commands. It cannot be confused with, or used as, the agent conversation surface.

### Adapter Requirements

- Codex uses its structured app-server JSON-RPC interface for threads, turns, approvals, history, and streamed events. It must not launch the interactive Codex TUI.
- A deterministic fake adapter exercises the complete normalized event lifecycle in tests and in browser-only visual development.
- The adapter registry can reject duplicate IDs, report unavailable binaries, and add a new adapter without modifying session-state or GUI reducers.
- Claude and other future harnesses have isolated adapter boundaries; incomplete adapters are reported as unavailable instead of falling back to showing their TUIs.

### Required Tests

- Unit tests cover normalized event validation, reducer state transitions, streaming message assembly, tool-call lifecycle, approval lifecycle, unknown-event preservation, and capability negotiation.
- Persistence tests prove ordered event replay reconstructs the same structured conversation.
- Adapter contract tests run the same lifecycle suite against the fake adapter and Codex adapter framing/fixtures.
- UI tests prove that Agent renders structured primitives and contains no terminal/xterm surface.
- E2E: create workspace → start Codex structured session → send a message → observe structured streamed response/tool activity → stop → restart → history remains.
- E2E: open Terminal separately → run `pwd` → return to Agent → structured conversation is unchanged and no terminal content leaked into it.
- `rg` verification finds no `TerminalPane` or xterm import reachable from the Agent view.

## Workspaces and Safety

- The macOS app opens into a dark control-room interface with project/workspace navigation, structured session tabs, metrics, and a composer.
- Local Git repositories, city-named worktrees, branches, sessions, normalized events, and lifecycle events persist in SQLite.
- Multiple structured sessions can share a workspace without sharing adapter process state.
- Session states are explicit: idle, working, waiting, ready, stopped, and failed.
- Dirty or active workspaces cannot be archived silently; preserved branches remain recoverable.
- Usage/context precision is labeled as reported, measured, or estimated.

## Verification Commands

- `bun run check` passes TypeScript and Rust checks.
- `bun run test` passes reducer, GUI, protocol, registry, persistence, and Git/worktree tests.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace live_app_server_emits_a_structured_turn -- --ignored` passes against an installed authenticated Codex binary.
- `bun run tauri build --debug --bundles app` produces a launchable `Bridge.app`.
- Packaged-app QA proves Agent renders structured history and provider errors, while Terminal is a distinct tab.
- `curl -s http://127.0.0.1:4317/health` returns health and adapter capability descriptors.

## Explicit v0 Boundaries

- Slack, GitHub webhooks, IMAP, Notion sync, Keychain secret brokering, nightly automation, and phone push are represented by typed extension points and native-pane placeholders, but are not connected to live external accounts in this first Conductor-clone slice.
- Usage/context values are estimates unless a harness emits provider-reported data; the UI must label the source.
- Code signing, notarization, auto-update, and public distribution are outside this local first build.
