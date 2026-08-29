# feat/343-github-surface-actions — Test Contract

Slice 4 of the Native GitHub surface (#339): the four mutating actions — reply to a
review comment, approve / request changes, re-run failed checks, and merge — each
behind a native confirmation, executed through `gh` argv arrays, gated by a
per-action approval so nothing mutates without an explicit user decision.

Locked before implementation. The read surface (slices 1–2) and CI polling (slice 3)
are already on `main`; this slice only adds write paths and their confirmation chrome.

## Design decisions (locked)

- **One write method, one small read method.** `github/act` performs every mutation;
  `github/merge_config` reads the repo's allowed merge strategies so the merge dialog
  can offer only those. No other existing read method changes shape.
- **The approval gate is server-side, not client trust.** `github/act` carries a
  `confirmed` boolean representing the native confirmation's outcome. `github_policy`
  evaluates it *before* the surface is touched: a denied action returns
  `{ executed: false }` and spawns zero subprocesses. The frontend only ever calls
  `github/act` after the user confirms, but the backend refuses to mutate on a denial
  regardless.
- **`gh` is authority.** Branch-protection / required-review / conflict refusals from
  `gh` surface verbatim; Bridge never re-interprets or retries a refused merge.
- **No repo argument from callers.** Repository identity comes from the workspace's Git
  config, exactly as the read surface already resolves it.
- **Merge default strategy.** GitHub's REST API does not expose a per-repo *default*
  merge method, only which are *allowed*. Bridge preselects `squash` when allowed, else
  `merge`, else `rebase` — the community-common default. A response that allows none is
  an unavailable configuration, never a fabricated merge choice.
- **Reply attribution.** This surface posts *ordinary user replies* (typed into the
  composer) verbatim — they are untouched, per the issue. The automated
  `_Created by [Claude](https://claude.com/claude-code)_` sign-off applies to the
  separate automated-fix path, not to a human's composer reply.

## Functional Behavior

- `github/merge_config { workspaceId }` → `{ strategies: { merge, squash, rebase }, defaultStrategy }`.
  - Reads `gh api repos/{owner}/{name}` (`--hostname` for non-github.com), maps
    `allow_merge_commit` / `allow_squash_merge` / `allow_rebase_merge`.
  - `defaultStrategy` = squash if allowed, else merge, else rebase. No allowed strategy
    returns a clear typed configuration error.
- `github/act { workspaceId, action, confirmed }` → `{ executed, message }`.
  - `confirmed == false` → `{ executed: false, message: "Declined: <statement>" }`, **zero `gh` spawns**.
  - `action = merge { number, strategy }` → `gh pr merge <n> --repo <sel> --{merge|squash|rebase}`.
  - `action = review { number, event, body }`:
    - `approve` → `gh pr review <n> --repo <sel> --approve` (+ `--body <body>` iff body non-empty).
    - `requestChanges` → `gh pr review <n> --repo <sel> --request-changes --body <body>`.
    - `comment` → `gh pr review <n> --repo <sel> --comment --body <body>`.
  - `action = reply { number, commentId, body }` →
    `gh api [--hostname <host>] --method POST repos/{owner}/{name}/pulls/{n}/comments/{commentId}/replies -f body=<body>`;
    typed reply text is passed through verbatim.
  - `action = rerun { number }` → resolve the current PR head SHA, `gh run list --repo <sel> --commit <sha>
    --json databaseId,status,conclusion --limit 20`, then `gh run rerun <id> --repo <sel> --failed`
    for each run whose conclusion is a failure. Only runs for the current head commit are eligible; a
    `null` conclusion is an active run, not a parser failure. No failed runs → `{ executed: true, message: "No failed runs to re-run." }`, no rerun spawn.
  - After a rerun attempt, the surface invalidates cached PR resources and re-arms slice-3 polling for the PR
    (re-list + `GithubPoller::watch`) so the rollup flips back to "running" and is watched.
- Every `gh` write is an argv array; identifiers come from workspace state or prior `gh` JSON, never interpolated into a shell.

## Unit Tests (Rust, `bridge-core`)

`github_surface` write paths, with the existing fake-`gh` shim extended to log argv and
branch on `pr merge` / `pr review` / `api ... replies` / `run list` / `run rerun` / `api repos`:

- `merge_config_reports_allowed_strategies_and_default` — parses allow_* booleans; default is squash when allowed.
- `merge_sends_the_selected_strategy_flag` — `--squash` (and `--merge`, `--rebase`) reach `gh pr merge` exactly; argv asserted from `invocations.log`.
- `merge_propagates_a_branch_protection_refusal_verbatim` — shim exits nonzero with a
  branch-protection stderr; `GithubSurfaceError::CommandFailed` carries the stderr unchanged; no retry (single `pr merge` invocation).
- `review_builds_the_correct_event_argv` — approve omits `--body` when empty; request-changes includes `--request-changes --body`; comment includes `--comment --body`.
- `reply_posts_to_the_review_comment_replies_endpoint` — argv targets `.../comments/<id>/replies` with `-f body=`.
- `reply_uses_workspace_hostname_for_github_enterprise` — a GHES remote sends `gh api --hostname <host>`.
- `completed_action_invalidates_all_cached_pull_request_resources` — an immediate list/detail/check/thread refresh runs fresh `gh` reads after a write.
- `rerun_acts_only_on_failed_runs` — `run list` fixture with one current-head failed + one passing + one active-null run ⇒ exactly one `run rerun <failedId> --failed`; a fixture with no failed runs ⇒ zero `run rerun` spawns.
- `rerun_skips_stale_branch_runs_and_refreshes_after_partial_failure` — old-head runs are ignored; if a later rerun refusal follows an earlier success, caches still invalidate and polling re-arms.

`github_policy`:

- `a_denied_action_authorizes_to_denied` / `a_confirmed_action_authorizes_to_approved`.
- `describe_states_the_exact_operation_and_target` — e.g. `merge PR #328 (squash) on owner/repo`.

Policy replay (`testing/fixtures/github-act-policy.json`):

- `github_act_policy_replay_covers_approve_and_deny_per_action_kind` — replays approve+deny
  for merge / review / reply / rerun; asserts each expected decision and that **no fake-`gh`
  binary is invoked on any denied case** (the api-level gate returns before the surface is touched).

Protocol (`bridge-protocol`):

- `github_act_payloads_round_trip_camel_case_and_reject_unknown_fields` — params tagged
  action enum round-trips; `deny_unknown_fields` rejects extras; merge-config result round-trips.

## Integration / Functional Tests

- `bridged` dispatch: `MethodName::GithubAct` / `GithubMergeConfig` decode params and call the api body (covered by the shell handler-registry parity test + a dispatch smoke where practical).
- Protocol artifact regeneration is drift-clean: `cargo test -p bridge-protocol`.
- api-level `github_act` deny path: with a fake `gh` on PATH, a `confirmed:false` merge returns `executed:false` and the fake logs nothing.

## Frontend Tests (vitest / jsdom)

`src/components/PullRequestView.test.tsx`:

- `merge dialog offers only allowed strategies and preselects the default` — mock
  `githubMergeConfig` returns `{ merge:true, squash:true, rebase:false, defaultStrategy:"squash" }`;
  rebase is absent, squash is checked.
- `the chosen strategy is what github/act receives` — pick `merge`, confirm ⇒ `githubAct`
  called with `{ kind:"merge", number, strategy:"merge" }, confirmed:true`.
- `cancel executes nothing` — open a confirm, click Cancel ⇒ `githubAct` spy never called.
- `a gh refusal renders verbatim` — `githubAct` rejects with a branch-protection message ⇒
  the exact text appears in the panel; PR state is not mutated optimistically.
- `reply composer submits the reply action` — type into a thread's composer, confirm ⇒
  `githubAct` called with `{ kind:"reply", number, commentId, body }`, preserving leading and trailing whitespace.
- `approve and request-changes submit the review action` — request-changes requires a body.
- `re-run is offered for any rerunnable terminal conclusion` — failures, timeouts, and startup failures are actionable.

`src/components/GitHubPanel.test.tsx` (existing) stays green.

## Smoke Tests

- `bun run build` — green.
- `bun run test` — sidecar + vitest + `cargo test --workspace` green.
- `bun run check` — `tsc -b` + `cargo check --workspace` green.

## E2E Tests

N/A — no browser E2E harness in this repo. The jsdom component tests and the fake-`gh`
Rust tests together exercise the full click → `github/act` → argv path offline.

## Manual / cURL Tests

Run against a real repo with `gh` authenticated (`bun run tauri dev`):

1. Open a PR in the panel. Click **Merge** → dialog names `merge PR #<n> (<strategy>) on <owner>/<repo>`;
   only allowed strategies are selectable; the default is preselected. Cancel → nothing runs.
2. Confirm a merge on a branch-protected PR → the `gh` refusal text appears verbatim; the PR row is unchanged.
3. **Approve** / **Request changes** a PR → confirmation names the action; the review posts and the decision re-renders after refresh.
4. **Reply** to a review comment → confirmation names the thread; the reply lands on the correct thread and appears after refresh.
5. **Re-run failed checks** on a PR with a failed run → only failed runs re-run; the rollup returns to "running" and slice-3 polling resumes watching automatically.
