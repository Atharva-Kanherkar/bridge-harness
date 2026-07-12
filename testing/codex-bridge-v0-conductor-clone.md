# codex/bridge-v0-conductor-clone — Test Contract

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
- `bun test` passes unit and integration tests.
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
