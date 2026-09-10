# Bridge

Bridge includes a supervised [authenticated browser bridge](docs/authenticated-browser-bridge.md) for using a user-approved logged-in Chrome or Safari tab without copying browser credentials.

Bridge is a native macOS control room for supervised coding-agent work. It connects local Git repositories to structured Codex, Claude Code, and OpenCode sessions, isolates concurrent tasks in worktrees, and keeps durable local history so agent activity remains inspectable and recoverable.

Bridge is built for developers who want the speed of coding agents with explicit boundaries around files, processes, approvals, delegation, and session state.

> Early-stage software: Bridge is currently packaged for macOS 12 or later and is under active development.

## Download

macOS 12 or later (Apple Silicon). Open the `.dmg`, drag **Bridge** into Applications, then launch it.

- **Release:** [GitHub Releases](https://github.com/Atharva-Kanherkar/bridge-harness/releases) — look for `Bridge_0.5.2_*.dmg`
- Claude models need **Node.js 18+** on your `PATH`. Codex, Claude Code, and OpenCode stay optional: a missing CLI shows that adapter as unavailable instead of blocking startup.
- A notarized build should open without Gatekeeper blocking it. If you built from source yourself, the binary is ad-hoc signed and macOS will ask you to open it anyway.

The application is all rights reserved unless the maintainers publish a license.

## Highlights

- **Structured agent sessions** — Connect Codex through its `app-server` JSON-RPC protocol, Claude Code through the Agent SDK sidecar, and OpenCode through its headless server API. Bridge renders normalized messages, reasoning, plans, tool calls, approvals, file changes, errors, and artifacts in the desktop UI instead of embedding provider TUIs.
- **Git worktree isolation** — Create a task workspace with its own branch and worktree. Independent worker sessions can receive additional isolated worktrees when their write scopes overlap.
- **Durable conversation history** — Store immutable session entries in a local SQLite session forest. Rewind or fork the conversation branch without pretending that files, commits, or provider state were rewound.
- **Supervised orchestration** — Run policy-authorized workers with bounded concurrency, capability tiers, budgets, retries, approvals, and typed results.
- **Checkpointing and resume** — Preserve verified checkpoints and choose an appropriate restoration mode when a session is resumed: hot, provider-native, checkpoint-restored, or fresh.
- **Model and capability management** — Configure role-based model profiles, inspect provider usage and context health, and discover or manage supported skills and capability integrations.
- **Separate terminal access** — Use a workspace terminal for ad-hoc shell work without mixing terminal output into the agent conversation.
- **Local observability** — Inspect adapter availability, database paths, snapshots, and runtime health through the in-app health view and the local health endpoint at `http://127.0.0.1:4317/health`.

## Core concepts

### Projects, workspaces, and sessions

- A **project** is a validated local Git repository.
- A **workspace** is a task-specific checkout associated with a project. It owns a branch, worktree path, task status, and the sessions that operate on it.
- A **session** is one provider conversation. A workspace may have an orchestrator session and child worker sessions; a direct chat can also run without a repository.

These are related but independent records. Ending a session does not automatically discard a worktree, and changing a conversation branch does not change Git state.

### Structured adapters

An **adapter** translates a provider's native process and event protocol into Bridge's provider-neutral event model. The built-in adapters are Codex, Claude Code, and OpenCode. Each adapter reports its availability and capabilities before a session starts. If a structured adapter is unavailable or incomplete, Bridge surfaces that state rather than silently falling back to a terminal UI.

### Three trees

Bridge keeps three kinds of hierarchy separate:

1. **Workspace tree:** repository → task worktree → optional worker worktree.
2. **Agent tree:** orchestrator → policy-authorized workers.
3. **Conversation tree:** immutable entries → the currently selected conversation branch.

An operation on one tree does not imply a matching operation on another. For example, forking a conversation changes the active history branch but does not undo filesystem changes.

### Session forest

The **session forest** is Bridge's append-only local conversation store. Each entry has an immutable identity, a parent entry, a semantic event kind, and visibility rules for context projection. Bridge also records Git `HEAD` and dirty-state evidence with controller-owned entries, allowing the UI to surface conversation/file divergence instead of hiding it.

The forest is local history, not a tamper-proof or replicated evidence ledger. See [`docs/session-forest.md`](docs/session-forest.md) for the storage model and projection rules.

### Orchestration and policy

The **orchestrator** can request work from child workers. The Rust policy engine decides whether to execute in the parent, reuse a compatible worker, spawn a new worker, queue, reject, or request approval. The policy owns the safety gates: capability availability, write scope, worktree isolation, concurrency, depth, retry limits, and per-turn budgets.

Worker results are typed and durable. A later worker can receive validated evidence from the parent's active branch rather than relying on an untracked paraphrase of another conversation. Read-only workers are checked for unexpected tracked-file changes; write-capable workers must operate within an approved scope.

### Checkpoints and restoration

A **checkpoint** is an immutable semantic boundary containing the decisions, risks, incomplete work, and file evidence needed to continue. Compaction preserves the original events and adds a verified boundary; it does not rewrite history. When a session returns, Bridge labels whether context was restored natively by the provider, projected from a checkpoint, or started fresh.

### Provenance and measurements

Bridge labels usage, context, and routing values by their source. A value may be provider-reported, measured from local events, or estimated. Routing recommendations are subordinate to deterministic policy: learning can rank eligible candidates, but it cannot grant permissions, widen a write scope, or bypass an approval.

## How a task flows through Bridge

1. Add a local Git repository as a project.
2. Create a workspace; Bridge creates the task branch and worktree.
3. Choose a harness and model profile. Bridge starts the structured provider process in the workspace.
4. Provider events are normalized into the session forest and rendered as native UI components.
5. The orchestrator may request worker help. Policy checks the request, write scope, budgets, leases, and worktree requirements before a worker is started.
6. Review approvals, changes, tool activity, and worker evidence in the session view.
7. Resume, rewind, or fork the conversation when needed. Stop sessions before archiving; dirty or active worktrees are not archived automatically.

## Architecture

```text
React 18 + TypeScript + Vite
            │
            │ Tauri commands and events
            ▼
Rust supervisor and policy engine
   ├── Codex adapter ──► codex app-server
   ├── Claude adapter ─► Claude Code sidecar
   ├── OpenCode adapter ► opencode headless server
   ├── Git worktree coordinator
   ├── Session forest and SQLite stores
   ├── Orchestrator, workers, and checkpoints
   └── PTY terminal and health reporting
```

The frontend lives in `src/`. The native side is a cargo workspace under `src-tauri/`: the `bridge-core` crate (`src-tauri/bridge-core/`) holds the Tauri-free runtime — provider supervision, persistence, routing, Git integration, policy, and the `BridgeCore` state — while the Tauri shell (`src-tauri/src/`) holds the IPC command wrappers and desktop wiring. The Claude sidecar lives in `sidecar/claude-agent/` and requires Node.js 18 or newer.

## Prerequisites

- macOS 12 or later
- [Bun](https://bun.sh/) for installing dependencies and running scripts
- Node.js 18 or newer for the Claude Code sidecar
- A stable Rust toolchain with Cargo
- Git with worktree support
- Optional provider CLIs and credentials:
  - `codex` for Codex sessions
  - `claude` for Claude Code sessions
  - `opencode` for OpenCode sessions

Bridge resolves provider binaries from `PATH` and common local installation locations. The application can still start when a provider is missing, but that adapter will be shown as unavailable until its binary and credentials are configured.

## Local development

Clone the repository and install dependencies:

```sh
git clone https://github.com/Atharva-Kanherkar/bridge-harness.git
cd bridge-harness
bun install
```

For fast frontend iteration, start the Vite development server:

```sh
bun run dev
```

This serves the React UI at `http://127.0.0.1:1420`. When it is running outside Tauri, the frontend uses local mock data for UI development.

To run the complete desktop application with the Rust shell, provider supervision, SQLite, Git worktrees, and PTY terminal:

```sh
bun run tauri dev
```

The package script stages the native browser host and daemon before starting
Tauri's dev-server readiness timer. This allows a cold Rust build to finish even
when it takes more than three minutes. Use this package script for desktop
development; direct Tauri CLI invocations require the native helpers to be staged
first.

When debugging provider discovery, confirm the binaries are visible to the same environment that launches the app:

```sh
command -v codex
command -v claude
command -v opencode
node --version
rustc --version
```

## macOS file access prompts

macOS gates `~/Desktop`, `~/Documents`, and `~/Downloads` behind per-app consent (TCC). The first time Bridge — or an agent process it supervises — touches a file inside one of those folders, macOS shows a "Bridge would like to access…" prompt, and a denied prompt turns into silent file-access failures later. Bridge's health response checks for the two situations that make this painful and shows a warning in the app for each:

- **A project or workspace registered inside a protected folder.** Every process in the chain needs its own grant, so prompts repeat per app and per folder. Keep repositories somewhere unprotected such as `~/Code`, or grant Bridge Full Disk Access under System Settings → Privacy & Security if you must work inside these folders.
- **An ad-hoc signed build.** macOS keys file-access grants to the app's code-signing identity. Locally built binaries (`bun run tauri dev`, `bun run tauri build --debug`) are ad-hoc signed by default — `codesign -dv` shows `Signature=adhoc` and no `TeamIdentifier` — and an ad-hoc identity changes on every rebuild, so yesterday's grants vanish and the prompts come back. Sign development builds with a stable identity (configure `signingIdentity` in the Tauri bundle settings, or re-sign the built app with your Apple Development certificate) to keep grants across rebuilds.

If prompts keep reappearing, address whichever of the two warnings the app shows. Stale per-app decisions can be cleared with `tccutil reset SystemPolicyDocumentsFolder <bundle-id>` (and the matching `SystemPolicyDesktopFolder` / `SystemPolicyDownloadsFolder` services) before relaunching.

## Verify and build

Run the checks used by the project before opening a pull request:

```sh
# TypeScript compilation and Rust checks
bun run check

# Frontend, sidecar, and Rust tests
bun run test

# Production frontend build
bun run build
```

Build a debug macOS application bundle:

```sh
bun run tauri build --debug
open src-tauri/target/debug/bundle/macos/Bridge.app
```

A production `.app` and `.dmg` (unsigned unless you set a Developer ID and notary credentials):

```sh
bun run tauri build
open src-tauri/target/release/bundle/macos/Bridge.app
```

Signed, notarized disk image for GitHub Releases (Developer ID Application certificate + App Store Connect API key or Apple ID app-specific password):

```sh
bun run release:dmg
```

The Tauri configuration targets a macOS `.app` and `.dmg` and uses `http://localhost:1420` for development.

## Repository layout

| Path | Purpose |
| --- | --- |
| `src/` | React UI, typed Tauri API boundary, event normalization, conversation projection, usage, and tests |
| `src-tauri/bridge-core/` | Tauri-free Rust runtime: adapters, process supervision, policy, orchestration, persistence, Git, worktrees, PTY, and health |
| `src-tauri/src/` | Tauri shell: IPC command wrappers, event emission, and desktop wiring around `bridge-core` |
| `sidecar/claude-agent/` | Node.js bridge for Claude Agent SDK sessions |
| `docs/` | Design notes for session history, delegation, compaction, local history, and adaptive learning |
| `testing/` | Acceptance contracts, regression notes, and replay fixtures |
| `assets/` | Application artwork and icons |

Useful design references:

- [`CHANGELOG.md`](CHANGELOG.md) — release notes for the downloadable app
- [`docs/protocol/README.md`](docs/protocol/README.md) — the versioned RPC contract, handshake, error codes, and generated client types
- [`docs/session-forest.md`](docs/session-forest.md) — immutable history, active branches, and divergence evidence
- [`docs/delegation-policy.md`](docs/delegation-policy.md) — routing, budgets, write isolation, approvals, and worker lifecycle
- [`docs/compaction-and-resume.md`](docs/compaction-and-resume.md) — checkpoint ownership and restoration modes
- [`docs/adaptive-learning.md`](docs/adaptive-learning.md) — role profiles, learning runs, and trigger safety
- [`docs/work-brief.md`](docs/work-brief.md) — connected integration activity from the past 24 hours, setup, and source timestamps

## Data and safety boundaries

Bridge keeps its primary state in a local SQLite database and records provider telemetry separately. On startup, the health response reports the exact database, telemetry, and history-snapshot paths. Provider processes run under Bridge supervision, and restart recovery checks the recorded process identity before terminating an orphaned process. Worktree archiving is conservative: active sessions and uncommitted changes prevent automatic cleanup.

Provider credentials remain managed by the provider installation and local machine. Bridge's credential and secret-interception layers pass references where required; they are not a hosted secret vault. Do not treat the local SQLite database or generated snapshots as encrypted backups.

## Current scope

The current build focuses on supervised local coding-agent sessions, isolated Git workspaces, durable session history, policy-controlled delegation, model routing, and native capability management. Hosted collaboration surfaces, webhooks, mail, Notion, phone notifications, and other external automation are outside the current desktop boundary.

## Contributing

Keep changes focused, preserve the Rust/React separation, and follow the repository guidance in [`AGENTS.md`](AGENTS.md). Styling is Tailwind CSS v4 only. Add colocated Vitest coverage for frontend logic and Rust tests for native behavior where applicable. Use Conventional Commits such as `feat:`, `fix:`, `docs:`, or `chore:`.

Before submitting a change, run:

```sh
bun run build
bun run test
```

## License

No license file is currently included in the repository. Treat the project as all rights reserved unless the maintainers provide separate written permission.
