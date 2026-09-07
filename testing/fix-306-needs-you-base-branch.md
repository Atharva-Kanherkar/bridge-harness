# fix/306-needs-you-base-branch — Test Contract

Issue: https://github.com/Atharva-Kanherkar/bridge-harness/issues/306
("Needs you not behaving correctly — straight from main")

## The bug

The Needs-you board shows a base-branch drift card with a fresh, successful
measurement ("artistant has drifted behind its base branch — 26 commit(s)
behind … measured 2 min ago"), yet clicking the card's action fails with the
raw error `{"code":1001,"kind":"git","message":"Git: fatal: not a git
repository (or any of the parent directories): .git"}`.

A card cannot claim a fresh measurement of a directory and then fail to
measure that same directory. Three defects combine to produce this:

1. **Directory mismatch (root cause).** The board's cached reading and the
   background observer measure the workspace root (`workspaces.path`,
   `work_observation.rs`), but the card's actions (`workspace_base_divergence`,
   `refresh_workspace_base`) resolve the directory via
   `repository_path_for_session` = `COALESCE(s.cwd, w.path)` — the session's
   own cwd wins. A workspace session's cwd can be a non-repository directory
   (chat scratch dir; also persisted back onto the row at turn start), so the
   action runs git somewhere the fact was never measured. The in-chat
   `warn_on_stale_base` has the same mismatch against the board.
2. **Raw error envelope.** In daemon-host mode, errors cross the Tauri
   boundary as the JSON string `{"code":1001,"kind":"git","message":"…"}`
   (`daemon_host.rs::host_error_envelope`), and `errorMessage()` renders that
   JSON verbatim instead of the human message inside it.
3. **Unfriendly refusal.** `fast_forward_to_base` runs `ensure_clean` before
   any repository check, so a non-repository directory surfaces raw git
   stderr instead of an explanation.

## Functional Behavior

- Base-branch facts describe the **workspace root**. Every code path that
  measures or mutates base-branch state for a session that belongs to a
  workspace must resolve `workspaces.path` first, falling back to the
  session cwd only for direct chats (no workspace row):
  - `api::workspace_base_divergence` ("Measure again")
  - `api::refresh_workspace_base` ("Fast-forward")
  - `live_turn::warn_on_stale_base` (in-chat warning)
- A session whose cwd is not a git repository, but whose workspace root is,
  gets a successful measurement of the workspace root — not a git error.
- A direct chat (no workspace row) keeps measuring the session cwd —
  unchanged behavior.
- `fast_forward_to_base` on a directory that is not a git repository refuses
  with a human explanation (`BridgeError::Invalid`), not raw git stderr.
- `errorMessage()` unwraps the daemon-host error envelope: given
  `{code, kind, message}` (object) or the same shape as a JSON string, it
  returns `message`. Plain strings and `Error` objects pass through
  unchanged.

## Unit Tests

Rust (`bridge-core`):
- `base_branch_path_prefers_the_workspace_root_and_falls_back_to_cwd` —
  store resolver returns `workspaces.path` for a workspace session, `s.cwd`
  for a direct chat, and `None` for an unknown session.
- `a_measurement_for_a_workspace_session_measures_the_workspace_root` —
  session cwd points at a non-repo scratch dir, workspace root is a real
  repo: `api::workspace_base_divergence` succeeds and the write-through cache
  holds the workspace root's reading (reproduces the issue end-to-end).
- `a_fast_forward_refuses_a_directory_that_is_not_a_repository` —
  `git::fast_forward_to_base` on a plain directory returns
  `BridgeError::Invalid` with a human message, not `BridgeError::Git`.

TypeScript (`src/errors.test.ts`):
- unwraps a daemon-host envelope object `{code, kind, message}` → `message`
- unwraps the same envelope arriving as a JSON string
- passes plain strings and `Error.message` through unchanged

## Integration / Functional Tests

- The existing write-through test
  (`a_divergence_reading_the_user_asked_for_is_written_through`) must keep
  passing — direct-chat behavior is unchanged.
- Existing board projection tests must keep passing (store-only read path
  untouched).

## Smoke Tests

- `cargo test -p bridge-core base_branch` and the new/updated test names pass.
- `bun run test` (vitest + cargo) green.
- `bun run build` and `bun run check` green.

## E2E Tests

N/A — no automated E2E harness for the desktop app. Manual repro below
stands in.

## Manual / cURL Tests

Repro of the original issue (before fix): create a workspace whose session
cwd is a non-repo directory (e.g. start a chat from `~/`), get the workspace
>20 commits behind its base, open Work → Needs you → click the drift card's
action → raw git error appears while the card claims a fresh measurement.

After fix: the action measures/fast-forwards the workspace root; on failure
the card shows a human-readable sentence (no JSON envelope).
