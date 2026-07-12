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

## Functional Behavior

- The macOS app opens into a dark, monospace control-room interface with an always-visible usage rail, project/workspace sidebar, live session pane, and composer.
- A user can add a local Git repository and see it persist after restart.
- A user can create an isolated workspace backed by a Git worktree and a dedicated branch. Workspaces use memorable city names plus their branch/task title.
- A user can choose Claude Code, Codex, or a shell and start a real interactive PTY session inside the selected workspace.
- PTY output streams into the selected session in real time; composer input and terminal keystrokes are written back to that same PTY.
- Multiple workspaces and sessions may run concurrently without sharing process state or working directories.
- Session state transitions are explicit: idle, working, waiting, ready, stopped, and failed. A stopped or failed session can be restarted.
- The UI surfaces repository, branch, dirty-file count, changed-line summary, process state, elapsed time, context estimate, and usage estimate without pretending provider precision.
- A user can create a second agent session inside the same workspace when agents should share branch/files.
- Workspace/session events are appended to local durable storage and replayed to a newly connected Deck client.
- Destructive actions require confirmation. A running or dirty workspace cannot be silently deleted.
- If a requested harness binary is unavailable, the app reports a useful error and leaves the workspace recoverable.

## Unit Tests

- Workspace name allocator returns stable, unique city names and safe branch slugs.
- Repository and worktree validation rejects non-Git paths and unsafe branch/path input.
- Session state reducer accepts valid transitions and rejects stale or invalid transitions.
- Usage/context parsers label values as reported, measured, or estimated.
- Event store appends ordered events and can reconstruct projects, workspaces, and sessions after restart.
- Command builder resolves Claude, Codex, and shell commands without shell interpolation.

## Integration / Functional Tests

- Daemon health endpoint returns success and reports detected harnesses.
- Adding a temporary Git repository persists it in SQLite.
- Creating a workspace creates a branch and worktree at the returned path.
- Starting a shell PTY emits session lifecycle and terminal-output events over WebSocket.
- Sending input over WebSocket reaches the correct PTY and its output returns only to subscribed clients.
- Stopping a session terminates its process and records the terminal state.
- Two shell sessions in separate workspaces run concurrently and report distinct working directories.

## Smoke Tests

- `bun run check` passes TypeScript checks for all packages.
- `bun run test` passes frontend and native unit/integration tests.
- `bun run build` produces the frontend bundle and daemon artifacts.
- `bun run tauri build --debug` produces a launchable macOS application bundle.
- Launching the app shows the primary shell without console errors.

## E2E Tests

- First run → add repository → create city workspace → launch shell → run `pwd` → see workspace path in terminal.
- Create another workspace → launch another session → both remain independently selectable and alive.
- Launch installed Codex or Claude harness → see its interactive UI/output → type into it → stop it cleanly.
- Restart daemon/app → repository and workspace return, while previously running sessions are marked stopped rather than falsely alive.
- Resize to a compact desktop window → rail, sidebar, terminal, and composer remain usable without overlapping controls.

## Manual / cURL Tests

- `curl -s http://127.0.0.1:4317/health` returns `{ "ok": true }` plus harness availability.
- Add a disposable repository through the GUI and verify `git worktree list` includes the created workspace.
- Confirm destructive workspace removal is blocked while a session is running and warns when files are dirty.
- Verify keyboard paths: `⌘N` creates a workspace, `⌘K` opens the command palette, and `⌘1` focuses the first workspace.

## Explicit v0 Boundaries

- Slack, GitHub webhooks, IMAP, Notion sync, Keychain secret brokering, nightly automation, and phone push are represented by typed extension points and native-pane placeholders, but are not connected to live external accounts in this first Conductor-clone slice.
- Usage/context values are estimates unless a harness emits provider-reported data; the UI must label the source.
- Code signing, notarization, auto-update, and public distribution are outside this local first build.
