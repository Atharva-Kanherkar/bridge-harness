# Bridge Deck v0.1

Bridge is a local macOS control room for structured coding-agent sessions in isolated Git worktrees. Rust supervises authenticated Codex and Claude Code processes, owns deterministic delegation policy, and persists every conversation as an append-only session forest.

## What works

- Add any local Git repository.
- Create a dedicated branch and worktree per independent task.
- Run **Codex** through its structured `app-server` JSON-RPC interface, and **Claude Code** through its structured `stream-json` stdio protocol, using the credentials and binaries already on the Mac.
- Opening a workspace (or creating one) auto-starts the selected structured agent and sends the task prompt—no manual “Start agent” gate.
- See messages, reasoning, plans, commands, tools, file changes, approvals, errors, and artifacts as native GUI components. Agent TUIs are never rendered.
- Use a separate workspace Terminal tab for ad-hoc `zsh` commands without leaking terminal content into agent history.
- Run a persistent orchestrator with bounded, policy-authorized workers.
- Resume sessions through hot, provider-native, checkpoint-restored, or fresh restoration modes.
- Rewind or fork conversation history without changing files or Git state.
- Persist projects, workspaces, sessions, append-only forest entries, worker leases, usage, and lifecycle decisions in SQLite WAL mode.
- Refresh dirty-file and diff statistics while an agent works.
- Stop sessions and safely archive only clean, inactive worktrees. Branches are preserved.
- Inspect daemon and harness health at `http://127.0.0.1:4317/health`.

## Architecture

- `src/` — React + TypeScript session-forest UI, separate workspace terminal, and typed Tauri boundary.
- `src-tauri/src/` — Rust adapters, session forest, context projector, policy engine, worker supervisor/pool, compaction controller, SQLite migrations, Git worktrees, PTY, and health.
- `docs/session-forest.md` — persistence, branching, and UI projection.
- `docs/delegation-policy.md` — routing, budgets, write isolation, lifecycle, and cancellation.
- `docs/compaction-and-resume.md` — checkpoint ownership and restoration modes.
- `testing/` — the acceptance contract locked before implementation.

Bridge launches coding harnesses only through registered structured adapters. An incomplete adapter is reported unavailable instead of falling back to its TUI. The SQLite session forest is the sole conversation store; provider-native context and projected checkpoint context remain separate. Workspaces are validated Git roots, overlapping writers are queued or isolated, uncommitted worktrees are never archived, and usage/context values are labeled as reported, measured, or estimated.

Claude Code and Codex binaries are resolved from `PATH` plus common install locations (`~/.local/bin`, Homebrew) so the packaged macOS app still finds them when launched from Finder.

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
