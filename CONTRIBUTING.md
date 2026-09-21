# Contributing to Bridge

Thanks for helping improve Bridge — the control room for coding agents on macOS. This guide covers local setup, the commands we expect before a pull request, and how issues and commits are formatted.

## Prerequisites

- **macOS 12+** (Apple Silicon) for the full desktop app; Linux can run frontend and Rust checks in CI
- **[Bun](https://bun.sh)** — package manager and script runner for the repo
- **Rust** (stable) and **Xcode Command Line Tools** — for `src-tauri/` and the Cargo workspace
- **Node.js 18+** on `PATH` — required for Claude sessions (Claude Agent SDK sidecar)
- **Git** — Bridge is built around real repositories and worktrees

Optional but useful for day-to-day work: provider CLIs (`codex`, `claude`, `opencode`, etc.). A missing binary shows that adapter as unavailable; it does not block startup.

## Get the repo running

```bash
git clone https://github.com/Atharva-Kanherkar/bridge-harness.git
cd bridge-harness
bun install
```

| Command | What it does |
| --- | --- |
| `bun run dev` | Vite dev server at http://127.0.0.1:1420 — frontend on mock data, no Rust shell |
| `bun run tauri dev` | Full Tauri 2 desktop app: Rust runtime, SQLite, adapters, PTY |
| `bun run check` | `tsc -b` plus `cargo check --workspace` |
| `bun run build` | Typecheck and production frontend build |
| `bun run test` | Sidecar, Vitest, and Cargo tests |

Run **`bun run build`** and **`bun run test`** before opening a PR. Both must be green.

Architecture and agent-specific conventions live in [`AGENTS.md`](AGENTS.md) (Tailwind CSS v4 only, colocated tests, layout patterns). Read it before touching `src/` or styling.

## Pull requests

1. **Branch** from `main` with a focused change set — no unrelated refactors bundled in.
2. **Title** — [Conventional Commits](https://www.conventionalcommits.org/) form: `feat:`, `fix:`, `docs:`, `chore:`, etc. CI may enforce this on the PR title.
3. **Description** — use the [pull request template](.github/PULL_REQUEST_TEMPLATE.md): summary, linked issue when there is one, and the checklist.
4. **Tests** — add or update colocated `*.test.ts(x)` for frontend logic; Rust tests next to code or under `src-tauri/*/tests/` as appropriate.
5. **Docs** — update user-facing or contributor docs when behavior or setup changes.

Maintainers review for correctness, safety boundaries (policy, worktrees, credentials), and fit with existing patterns.

## Commits

Use Conventional Commits in commit messages and PR titles:

- `feat:` — user-visible capability
- `fix:` — bug fix
- `docs:` — documentation only
- `chore:` — tooling, deps, repo maintenance
- `refactor:`, `test:`, `ci:` — as usual

Describe the change; do not cite issue or PR numbers in commit messages unless the project explicitly asks for it.

## Issues

Every issue is read by **humans** and **agents**, so the body has two required sections:

1. **`## For humans`** — TL;DR, diagram, architecture, decision (short)
2. **`## For agents`** — repro, `file:line`, files to touch, acceptance criteria, tests (exhaustive)

The full rule, gate behavior, and escape hatch are in [`docs/issue-format.md`](docs/issue-format.md). Start from a template under [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/) (`bug`, `feature`, or `audit`). Issues missing either section are closed automatically until the body is fixed.

**Etiquette:** search existing issues first; one problem per issue; include versions and repro steps in **For agents**; keep **For humans** scannable.

## Security

Do not open public issues for exploitable security problems. See [`SECURITY.md`](SECURITY.md) for how to report vulnerabilities.

## License

By contributing, you agree that your contributions are licensed under the same [MIT License](LICENSE) as the project.
