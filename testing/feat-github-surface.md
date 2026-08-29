# feat/github-surface-core — Test Contract

Implements #340, the first read-only `bridge-core` slice of the native GitHub
surface epic #339. This contract covers the module's data shapes so later
protocol, polling, action, and UI slices can build on a stable Rust boundary.

## Functional Behavior

- `GithubSurface::discover` resolves `gh` without making GitHub authentication a
  startup precondition. Discovery reports exactly one typed state:
  `Available`, `NotInstalled`, or
  `NotAuthenticated { remediation: "gh auth login" }`.
- Availability can be refreshed on demand. A missing executable, failed auth
  probe, failed subprocess, or malformed response returns a typed state/error;
  none panics.
- Every read is scoped from a caller-provided workspace/worktree path. Public
  read methods never accept a free-form repository selector.
- Repository resolution uses the current branch's push remote (falling back to
  its upstream remote), then `gh repo set-default --view`, then `origin`.
  GitHub HTTPS, SSH, and scp-style remote URLs normalize to a typed repository
  selector. A linked Git worktree resolves to the same repository as its parent
  checkout when they share the same remotes/default.
- `list_prs` invokes `gh pr list --json` with an explicit field list and returns
  typed summaries containing PR identity, title, branches, author, state, draft
  status, review decision, mergeability, URL, and a normalized CI rollup.
- `pr_detail` invokes `gh pr view --json` with an explicit field list and returns
  the summary fields plus body, base branch, and merge-state status.
- `pr_checks` invokes `gh pr checks --json` with an explicit field list and
  normalizes `gh`'s combined check `state` into a typed execution status and
  optional conclusion while retaining the log URL and workflow name.
- `pr_review_threads` invokes `gh api graphql` using a constant query and typed
  variables. It returns thread resolution/location plus typed comments and
  reply relationships; remote bodies are retained as data, never executed or
  rendered as HTML by this module.
- PR numbers are passed as individual argv values. Repository selectors come
  only from Git configuration/remotes or `gh`'s prior output. No invocation is
  assembled through a shell.
- Successful reads are cached per repository, resource kind, and PR number for
  a short TTL. Repeating the same read inside the TTL returns an equivalent
  clone without spawning the resource command again; different resources do
  not alias. Failed reads are not cached.
- Missing required fields, wrong field types, partial GraphQL envelopes, null
  review-thread nodes, unknown PR state values, and unrecognized completed
  check states produce `GithubSurfaceError::MalformedResponse`; they never
  become empty collections or fabricated defaults.

## Unit Tests

- `discovery_reports_available_for_an_authenticated_gh` — an executable fake
  `gh` whose auth probe succeeds reports `Available`.
- `discovery_reports_not_installed_without_gh` — an explicitly empty discovery
  path reports `NotInstalled`, and a read returns a typed unavailable error.
- `discovery_reports_signed_out_with_exact_remediation` — a fake `gh` whose
  auth probe fails reports `NotAuthenticated` and exactly `gh auth login`.
- `list_prs_parses_every_interesting_state` — the shared PR fixture yields open,
  draft, changes-requested, failing-CI, and merge-conflict summaries with the
  expected typed rollups.
- `pr_detail_parses_body_review_and_mergeability` — detail fields and normalized
  enums match the fixture.
- `pr_checks_normalize_states_and_log_urls` — queued/running and completed
  success/failure checks map to typed status/conclusion values and preserve log
  URLs.
- `pr_review_threads_parse_comments_and_replies` — GraphQL threads retain
  resolved/outdated state, path/line information, authors, bodies, URLs, and
  reply-to IDs.
- `repository_resolution_follows_push_default_origin_order` — table-driven real
  Git repositories prove push/upstream remote outranks the `gh` default, which
  outranks `origin`.
- `linked_worktree_resolves_like_its_parent` — a real linked worktree and parent
  checkout produce the same typed repository.
- `malformed_or_partial_json_is_a_typed_error` — malformed JSON and a syntactic
  object missing required fields both return `MalformedResponse`.
- `same_resource_is_spawned_once_inside_the_ttl` — two identical PR-list reads
  create one `gh pr list` log entry; a different read creates its own entry.
- Tests use executable fake-`gh` shims discovered from an injected PATH and the
  canned files under `testing/fixtures/github/`; they require no network or live
  GitHub authentication.

## Integration / Functional Tests

- `github_surface` is exported from `bridge-core` and compiles as part of the
  workspace without adding protocol methods, Tauri commands, or frontend code.
- `cargo test -p bridge-core github_surface` runs the module against real local
  Git repositories and fake `gh` processes only.
- Existing `bridge-core` tests remain green.

## Smoke Tests

- `cargo test -p bridge-core github_surface` passes.
- `cargo test -p bridge-core` passes.
- `bun run build` passes.
- `bun run test` passes, including the full frontend and Rust suites.

## E2E Tests

N/A — this slice adds no protocol route or UI. End-to-end GitHub journeys belong
to later epic slices; subprocess-to-parser behavior is covered with fake-`gh`
functional tests here.

## Manual / cURL Tests

N/A — no HTTP endpoint is introduced, and automated tests must not depend on a
developer's GitHub login. Optional manual review may instantiate the surface in
a scratch Rust example and compare the typed output with `gh pr list`, but it is
not a release gate.

---

## Epic close-out — manual pass (record before closing #339)

Slice 5 rehomed the surface into the GitHub dock pane and added the workflow
glue (CI-finished toasts, jump-to-diff, PR checkout into a task worktree). The
full loop below is the epic's release gate; run it in the desktop app against a
real repository and check the boxes with the date.

- [ ] Open the GitHub dock pane (⌥⌘7 or the sidebar PR row) → the PR list
      renders with review + rollup chips.
- [ ] Watch a running PR: rollup flips live; on the terminal state exactly one
      CI toast appears; clicking it lands on that PR in the pane.
- [ ] Click a review comment's `path:line` → the editor opens at that line; on
      a workspace that is not on the PR head branch, the "isn't checked out
      here" hint shows and nothing crashes.
- [ ] "Check out" the PR → a task worktree on the head branch appears in the
      workspace tree; repeating it reuses the same worktree; the current
      checkout keeps its dirty state.
- [ ] Fix the flagged line in the task worktree, push, "Re-run failed" from the
      pane, watch checks go green, and merge from Bridge behind the native
      confirmation.
- [ ] `bun run dev` (mock mode): list/detail render, checkout narrates fresh
      then reused, and the simulated CI completion (~6s after the first PR list
      read) exercises the toast → deep-link path.
