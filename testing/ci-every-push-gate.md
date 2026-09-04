# ci/every-push-gate — Test Contract

Source: the `[ci] Run typecheck + build + tests on every push` tracker issue
(strict GitHub Actions gate; red `main` impossible to miss; PR required checks).

This is a CI-pipeline-only change: one new workflow file
`.github/workflows/ci.yml`. No application code is touched. The contract below
describes what the pipeline must do and how each claim is verified before the
PR opens.

## Functional Behavior

- `.github/workflows/ci.yml` exists; `deploy-docs.yml` is byte-for-byte
  untouched (still manual `workflow_dispatch` only).
- Triggers: every branch push (`branches: ["**"]`, no tags) **and** every
  `pull_request`.
- Concurrency: one group per ref (`ci-${{ github.ref }}`),
  `cancel-in-progress: true` — a new push to the same branch cancels the
  superseded run.
- Least privilege: `permissions: contents: read` at workflow level.
- All jobs run on `ubuntu-latest` Linux runners. No macOS runners.
- Three parallel jobs:
  1. **Frontend** — bun install (frozen lockfile, cached), `bun run build`
     (= `tsc -b && vite build`, typecheck + build), `vitest run`.
  2. **Rust** — `cargo check --manifest-path src-tauri/Cargo.toml --workspace`
     and `cargo test --manifest-path src-tauri/Cargo.toml --workspace`, with
     `~/.cargo` + workspace `target/` cached (Swatinem/rust-cache keyed on
     Cargo.lock). Because `tauri::generate_context!` (src-tauri/src/lib.rs)
     requires `frontendDist: "../dist"` to exist at compile time and the Rust
     job does not build the frontend, the job stubs an empty `dist/` first.
  3. **Sidecar** — `npm test --prefix sidecar/claude-agent` (= `node --test
     test/*.mjs`), with the Claude Agent SDK resolved from the root bun
     workspace install (pinned by `bun.lock`, not freshly floated).
- No `cargo fmt --check` / `clippy -D warnings` in this PR — the issue frames
  those as a follow-up strictness step; adding them now would gate the
  pipeline on pre-existing warnings it never signed up for.

### Scope addition (recorded after the first CI runs)

The gate's acceptance — "cargo test green on the runner" — exposed two
product-code portability bugs that macOS never exercises. Fixing them is in
scope for this branch, because a strict gate that is red on day one fails the
issue's own acceptance criteria:

1. `adapters::terminate_process_group` shelled `kill -TERM -<pgid>`; procps-ng
   (Linux) silently no-ops that form (rc=0, no signal delivered), while macOS
   tolerates it. Proven empirically in an ubuntu:24.04 container: bare form
   leaves the process alive, `kill -TERM -- -<pgid>` works. Fix: emit `--`
   before the target. Eight lifecycle tests across acp_session, adapters,
   process_ledger, runtime, and session_supervisor failed on CI through this
   single root cause.
2. `api::open_terminal` hardcoded `zsh -l`; ubuntu-latest has no zsh, so the
   two terminal tests panic with a Pty spawn error — and real Linux users
   without zsh could not open terminals at all. Fix: `login_shell()` picks
   zsh where present, else `$SHELL`, else bash (behavior unchanged on macOS).
3. The parent-death watchdog script (`adapters::PARENT_WATCHDOG_SCRIPT`)
   called bare `kill -TERM -- -$$` to signal its own process group.
   `/bin/sh` on Debian/Ubuntu is dash, and dash's *builtin* `kill` rejects
   the `--` separator before a negative PID ("Illegal number: -"), failing
   silently behind `2>/dev/null` — the group was never actually signaled on
   Linux. Fix: invoke `/bin/kill` by full path. Separately,
   `watchdog_reaps_child_and_group_mates_after_supervisor_sigkill` used
   `set -m` to give the simulated watchdog its own process group; shell job
   control silently no-ops without a controlling TTY, which a CI runner
   never has, so the watchdog stayed in the supervisor's group with no
   group of its own to reap. Fix: prefer `setsid` (Linux) and fall back to
   `set -m` for interactive local development (macOS has no `setsid`).
   Reproduced deterministically in an ubuntu:24.04 container before fixing.

## Static Validation (in place of unit tests — no runtime code is added)

- `ci.yml` parses as valid YAML (`python3 -c "import yaml; yaml.safe_load(...)"`).
- Workflow structure is valid per GitHub Actions schema: `on` triggers,
  `permissions`, `concurrency`, three jobs each with `runs-on` + `steps`.
- Every action ref is a pinned major tag (`actions/checkout@v4`,
  `actions/setup-node@v4`, `actions/setup-bun@v2`, `actions/cache@v4`,
  `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`).
- `git diff main -- .github/workflows/deploy-docs.yml` is empty.

## Integration / Functional Tests

Each command the pipeline will run must pass locally on this exact branch
(fresh `origin/main`) before the PR opens:

- `bun install --frozen-lockfile` — exit 0, lockfile not modified
  (`git diff --exit-code bun.lock`).
- `bun run build` — `tsc -b` + `vite build` succeed, `dist/` produced.
- `bunx vitest run` — web test suite green.
- `npm test --prefix sidecar/claude-agent` — node --test suite green.
- `mkdir -p dist && cargo check --manifest-path src-tauri/Cargo.toml
  --workspace` — exit 0 (validates the dist-stub ordering assumption).
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` — green.

## Smoke Tests

- `gh workflow view ci.yml` shows the workflow with triggers push + pull_request.
- `gh api` / workflow file listing confirms `ci.yml` and `deploy-docs.yml`
  both exist, the latter unmodified.

## E2E Tests

- Push the branch → the PR shows all three checks (Frontend, Rust, Sidecar)
  running and passing; a second push to the same branch cancels the first
  run (concurrency) and the replacement run passes.

## Manual / cURL Tests

```sh
# Workflow registered with the right triggers
gh workflow view ci.yml --repo Atharva-Kanherkar/bridge-harness

# Required-check names once green (branch protection step, post-merge, manual):
#   Frontend / Rust / Sidecar  → Settings → Branches → main → required checks
gh pr checks <pr-number> --repo Atharva-Kanherkar/bridge-harness
```

Branch protection itself is a manual repo-settings step and intentionally out
of scope for this PR (documented in the PR body).
