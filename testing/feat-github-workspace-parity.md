# feat/github-workspace-parity — Test Contract

## Functional Behavior

- The GitHub dock is a repository workspace, not a pull-request-only screen:
  users can switch between Pull requests, Issues, and Repository without leaving
  Bridge.
- Pull request detail keeps the existing summary/actions and adds native tabs for:
  - Conversation: description, issue comments, and review threads render as inert
    text, with remote HTML never interpreted.
  - Changes: every changed file reports status/additions/deletions and renders its
    patch inside Bridge; binary or omitted patches show an explicit fallback.
  - Checks: running, passing, failing, cancelled, and empty check states remain
    visible without opening GitHub.
- Pull requests show their labels. Users can add or remove a repository label from
  the PR detail through the existing explicit confirmation overlay.
- Issues list open issues for the workspace repository (pull requests excluded).
  Issue detail shows the body, author, labels, and comments as inert text.
- Users can add or remove a repository label from issue detail through an explicit
  confirmation overlay. Cancelling performs no mutation.
- Repository overview shows the resolved owner/name, description, visibility,
  default branch, primary language, and current open issue/PR counts inside Bridge.
- While the selected PR has queued or in-progress checks, the dock refreshes its
  checks and list automatically on a bounded interval. Polling stops when checks
  become terminal, when the selection changes, or when the pane unmounts.
- Read failures render in the affected tab without erasing already loaded content.
  Mutation failures render verbatim and do not optimistically change labels.
- All GitHub commands remain argv-only `gh` invocations scoped to the workspace's
  resolved repository. No model call, iframe, new credential store, or external
  browser is introduced.

## Unit Tests

- `github_surface::pr_files_parse_text_and_binary_changes` — normalizes file
  status, counts, and optional patches.
- `github_surface::pr_conversation_parses_comments_and_labels` — returns ordinary
  PR comments and labels alongside the existing review threads.
- `github_surface::issues_exclude_pull_requests_and_parse_detail` — lists and opens
  repository issues with comments and labels.
- `github_surface::repository_overview_and_labels_are_typed` — parses repository
  metadata and available labels.
- `github_surface::label_actions_use_exact_argv_and_invalidate_reads` — add/remove
  label mutations target the selected PR or issue and invalidate affected caches.
- Protocol round-trip tests cover all added request/result/action variants and
  reject unknown request fields.
- Frontend helper tests cover check polling eligibility and changed-file patch
  presentation fallbacks.

## Integration / Functional Tests

- `GitHubPane.test.tsx` covers switching Pull requests → Issues → Repository.
- Opening a PR exposes Conversation, Changes, and Checks; changed-file patches and
  ordinary/review comments are visible, and remote HTML stays inert.
- A running check schedules live refresh; terminal checks cancel it; unmount leaves
  no timer or state update behind.
- Opening an issue shows its body/comments and label controls.
- PR and issue label add/remove flows show the exact confirmation, cancel is inert,
  confirm calls the typed action, and success refreshes detail.
- Per-tab read errors and mutation errors remain recoverable.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds, including Vitest, sidecar tests, and the Rust workspace.
- `git diff --check` reports no whitespace errors.

## E2E Tests

- N/A — GitHub authentication and live repository mutation are intentionally not
  exercised by an automated end-to-end test. The fake-`gh` Rust harness and mounted
  React integration tests cover the same command and UI boundaries deterministically.

## Manual / cURL Tests

- In `bun run dev` mock mode: open the GitHub dock, visit all three repository tabs,
  open a PR and inspect Conversation/Changes/Checks, then open an issue.
- In the Tauri app with authenticated `gh`: open a PR with running CI and verify the
  check status updates without pressing Refresh.
- Cancel a label mutation and verify no change. Confirm one add/remove label action
  and verify the refreshed labels match GitHub.
- N/A for cURL — the feature is a Tauri command surface, not an HTTP service.
