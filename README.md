# Bridge Deck v0.1

Bridge is a local macOS control room for running Claude Code, Codex, and shell sessions in isolated Git worktrees. This first vertical slice implements the Conductor-style foundation from the Bridge v0.3 blueprint: projects, city-named workspaces, parallel agents, live PTYs, persistent state, attention states, Git change summaries, and an always-visible usage/context rail.

## What works

- Add any local Git repository.
- Create a dedicated branch and worktree per independent task.
- Run real interactive `claude`, `codex`, or `zsh` sessions using the credentials and configuration already on the Mac.
- Run multiple sessions in one workspace when agents need the same branch and files.
- Switch sessions without losing terminal scrollback.
- Persist projects, workspaces, sessions, and lifecycle events in SQLite WAL mode.
- Refresh dirty-file and diff statistics while an agent works.
- Stop sessions and safely archive only clean, inactive worktrees. Branches are preserved.
- Inspect daemon and harness health at `http://127.0.0.1:4317/health`.

## Architecture

- `src/` — React + TypeScript Deck UI, xterm.js terminal, and typed Tauri API boundary.
- `src-tauri/src/` — Rust backend for SQLite, Git worktrees, PTY supervision, events, metrics, and health.
- `testing/` — the acceptance contract locked before implementation.

Bridge invokes harness binaries directly without shell interpolation. Workspaces are validated Git roots, uncommitted worktrees are never archived, and usage/context values are labeled as reported, measured, or estimated.

## Run and verify

```sh
bun install
bun run check
bun run test
bun tauri dev
```

Build the macOS application:

```sh
bun tauri build --debug
open src-tauri/target/debug/bundle/macos/Bridge.app
```

The current local debug bundle is generated at `src-tauri/target/debug/bundle/macos/Bridge.app`.

## v0 boundary

The service-specific panes from the broader blueprint—Slack, GitHub webhooks, mail, Notion, Keychain secret brokering, morning briefings, nightly automation, and phone push—are the next layers. This build establishes the real supervised-session and worktree primitives those panes dispatch into.
