# Contributing to Bridge

Thanks for helping improve Bridge — the control room for coding agents on macOS. This guide covers local setup, how we work in GitHub, and where to look for deeper conventions.

## Prerequisites

- **macOS 12+** (Apple Silicon) for the full desktop app; Linux can run frontend and Rust checks in CI-like environments.
- **[Bun](https://bun.sh)** — package manager and script runner for the repo.
- **Rust** (stable) and **Xcode Command Line Tools** — for the Tauri 2 shell and `src-tauri/` workspace.
- **Node.js 18+** on `PATH` — Claude sessions use the Node sidecar (`sidecar/claude-agent/`), not `claude -p`.
- **Git** — Bridge is built around real repositories and worktrees.

Provider CLIs (`codex`, `claude`, `opencode`, `cursor-agent`, `grok`) are optional locally. A missing binary shows that adapter as unavailable instead of blocking startup.

## Local setup

```sh
git clone https://github.com/Atharva-Kanherkar/bridge-harness.git
cd bridge-harness
bun install
```

`bun install` installs dependencies for the root app and the sidecar workspace.

## Development commands

| Command | What it does |
| --- | --- |
| `bun run dev` | Vite frontend at http://127.0.0.1:1420 (mock data, no Rust shell) |
| `bun run tauri dev` | Full desktop app: Rust runtime, SQLite, adapters, PTY |
| `bun run build` | `tsc -b` and `vite build` |
| `bun run test` | Sidecar tests, Vitest, and `cargo test --workspace` |
| `bun run check` | `tsc -b` plus `cargo check --workspace` |

Run **`bun run build`** and **`bun run test`** before opening a pull request. Both must be green.

Narrower runs (one frontend file, one Rust module, one sidecar test) are documented in [`CLAUDE.md`](CLAUDE.md).

## Code conventions

- **Frontend:** React 18, TypeScript, Vite. Style exclusively with **Tailwind CSS v4** (see [`AGENTS.md`](AGENTS.md) — no `tailwind.config.js`, no CSS-in-JS, one global stylesheet in `src/index.css`).
- **Native:** Tauri 2 shell in `src-tauri/`; behavior lives in the `bridge-core` and related crates. Protocol changes go through `bridge-protocol` and generated artifacts — never hand-edit generated schema files.
- **Tests:** Colocated `*.test.ts(x)` under Vitest; Rust `#[cfg(test)]` and integration tests as described in `AGENTS.md` and `CLAUDE.md`.
- **Commits:** [Conventional Commits](https://www.conventionalcommits.org/) — `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `ci:`, etc. PR titles follow the same pattern (see CI).

For agent- and contributor-facing UI and architecture rules, read **[`AGENTS.md`](AGENTS.md)** in full.

## Pull requests

1. Branch from `main` with a focused change set.
2. Use a **Conventional Commit** PR title (e.g. `fix: handle daemon startup race`, `feat(settings): add provider row`).
3. Fill out the [pull request template](.github/PULL_REQUEST_TEMPLATE.md): summary, linked issue if there is one, and the checklist.
4. Ensure `bun run build` and `bun run test` pass locally; CI runs the same gate on every push and PR.

Sign-in and credentials stay on the machine — Bridge does not collect provider secrets. Do not commit API keys, `.env` files with secrets, or local data directories.

## Issues

Every issue needs **two audiences** in one body:

1. **`## For humans`** — short TL;DR, architecture, diagram when it helps.
2. **`## For agents`** — repro, suspected `file:line`, files to touch, acceptance criteria, tests.

Issues missing either section are closed automatically. Full rules, templates, and the `format-exempt` escape hatch are in **[`docs/issue-format.md`](docs/issue-format.md)**. Start from a template under [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/) (`bug`, `feature`, or `audit`).

## Security

See **[`SECURITY.md`](SECURITY.md)** for how to report vulnerabilities.

## License

Contributions are accepted under the same [MIT License](LICENSE) as the project.
