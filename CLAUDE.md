# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Read [`AGENTS.md`](AGENTS.md) too — it carries the styling rules and UI conventions in full. The most important one: **this project styles exclusively with Tailwind CSS v4** (CSS-first config in `src/index.css`, no `tailwind.config.js`, no CSS-in-JS, no extra `.css` files, no inline `style` for anything a utility can express).

## Commands

```bash
bun install                # deps for the root app and the sidecar workspace
bun run dev                # Vite only, http://127.0.0.1:1420 (frontend runs on mock data)
bun run tauri dev          # full desktop app: Rust shell, SQLite, adapters, PTY
bun run check              # tsc -b + cargo check --workspace
bun run test               # sidecar node:test + vitest run + cargo test --workspace
bun run build              # tsc -b && vite build
```

`bun run build` and `bun run test` must both be green before a PR.

Narrower runs:

```bash
bunx vitest run src/components/WorkView.test.tsx        # one frontend file
bunx vitest run -t "name of the case"                   # one frontend case
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core policy::   # one Rust module
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --lib some_test -- --exact
node --test sidecar/claude-agent/test/briefing.mjs      # one sidecar file
```

Protocol artifacts are generated, and drift is test-enforced:

```bash
cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol
```

## Architecture

Bridge is a macOS Tauri 2 desktop app: a React 18 + TypeScript + Vite frontend over a Rust runtime that supervises Codex, Claude Code, and OpenCode sessions against isolated Git worktrees.

**Cargo workspace (`src-tauri/`), four crates plus the shell:**

- `bridge-core/` — the Tauri-free runtime and where nearly all behavior lives: adapters (`codex_adapter`, `claude_adapter`, `opencode_adapter`), process supervision, SQLite stores, `session_forest`, `policy`/`policy_coordinator`, orchestration and workers, `git`/`worktree_coordinator`, PTY, health. `bridge_core::api` holds the host-agnostic body of every protocol method.
- `bridge-protocol/` — the single source of truth for the RPC contract. `docs/protocol/schemas/` and `src/protocol/generated/protocol.ts` are generated from it and must never be hand-edited.
- `bridged/` — the daemon that owns a data directory: newline-delimited JSON-RPC 2.0 over a Unix socket, token handshake, event fan-out, `/healthz` and `/readyz` on 4318.
- `bridge-client/` — shared Rust client (handshake, gap-free `SessionEventStream`); ships the `bridge exec --json` CI one-shot.
- `src-tauri/src/` — the Tauri shell: thin `#[tauri::command]` wrappers that decode arguments and hand anything touching Git, processes, PTYs, or the network to `spawn_blocking`, then call `bridge_core::api`.

By default the desktop app runs as a *daemon client* and proxies every invoke to `bridged`; `BRIDGE_DESKTOP_HOST` selects `auto`, `daemon`, or the in-process `embedded` host. Exactly one owner per data directory, enforced by a file lease (`bridge_core::ownership`). See [`docs/protocol/README.md`](docs/protocol/README.md).

**Frontend (`src/`).** `App.tsx` owns app state and composes `BridgeSidebar`, `ComposerPill`, `AgentConversation`, `WorkView`, and dialogs. `src/api.ts` is the only Tauri round-trip: `call()`/`subscribe()` key off the generated method registry, so renaming a wire field breaks `bun run check` rather than a user session. Outside Tauri, `api.ts` serves mock data — that is what `bun run dev` and most component tests exercise. `src/types.ts` renames and refines generated types; it never restates them.

**Sidecar (`sidecar/claude-agent/`).** Claude runs through the Claude Agent SDK in a Node process, not `claude -p`. Keep Node 18+ on PATH. The Rust adapter resolves it via `BRIDGE_CLAUDE_SIDECAR`, then next to the executable, then the in-repo path.

**Three separate hierarchies** — conflating them is the recurring design bug: the workspace tree (repo → task worktree → worker worktree), the agent tree (orchestrator → policy-authorized workers), and the conversation tree (immutable forest entries → active branch). Forking a conversation does not undo filesystem changes; ending a session does not discard a worktree.

**The session forest** is append-only local history. Entries are immutable with a parent, an event kind, and visibility rules for context projection; compaction adds a verified checkpoint boundary rather than rewriting events. **The policy engine** owns every safety gate — capability tier, write scope, worktree isolation, concurrency, depth, retries, budgets, approvals. Learning and routing may rank eligible candidates; they can never grant a permission, widen a scope, or skip an approval.

Design references worth reading before touching those areas: [`docs/session-forest.md`](docs/session-forest.md), [`docs/delegation-policy.md`](docs/delegation-policy.md), [`docs/compaction-and-resume.md`](docs/compaction-and-resume.md), [`docs/managed-agent-runtimes.md`](docs/managed-agent-runtimes.md), [`docs/adaptive-learning.md`](docs/adaptive-learning.md), [`docs/worktree-lifecycle.md`](docs/worktree-lifecycle.md).

## Testing conventions

- Frontend: colocated `*.test.ts(x)` under Vitest. The default environment is node; component tests opt in per file with a `// @vitest-environment jsdom` docblock and mount through `react-dom/client` + `act` rather than Testing Library. Vitest excludes `.worktrees/**` — Bridge creates task worktrees inside the repo and their tests belong to other branches.
- Rust: inline `#[cfg(test)] mod tests` next to the code; integration tests in `bridge-client/tests/`, `bridged/tests/`, `src-tauri/tests/`. Replay fixtures live in `testing/fixtures/`.
- `testing/*.md` are per-feature test contracts, locked before implementation and named after the branch (`feat-work-facts-ui.md`). When starting a sizeable feature, write or read the matching contract first.
- Commits follow Conventional Commits and describe the change without citing issue or PR numbers.

## Environment

`BRIDGE_DATA_DIR`, `BRIDGE_DESKTOP_HOST`, `BRIDGE_DAEMON_BIN`, `BRIDGE_CLAUDE_SIDECAR`, `BRIDGE_BROWSER_EXTENSION`, `BRIDGE_WORKER_OUTPUT_DIR`. Provider CLIs (`codex`, `claude`, `opencode`) are optional — a missing binary shows its adapter as unavailable instead of blocking startup, so check `command -v` when debugging discovery. Vendor runtimes under `runtimes/` are pinned npm closures installed with `npm ci --ignore-scripts` and verified by receipt.

## PR review reply attribution

When replying to an inline PR review comment as an automated fix, sign off with `_Created by [Claude](https://claude.com/claude-code)_` — not "Addressed by Claude Code".
