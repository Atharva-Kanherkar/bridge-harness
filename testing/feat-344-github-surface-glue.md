# Test contract — github-surface 5/5: notifications, jump-to-diff, PR checkout (+ surface rehome)

Slice 5 of the native GitHub surface. Three glue features on top of slices 1–4,
plus one structural fix this slice carries: the surface moves out of the
sidebar popover (whose `fixed` overlay was trapped by the sidebar's CSS
transform and clipped by its scroll region — the "never opens correctly" bug)
into a first-class dock pane.

## Rust — bridge-core

### `github_poll` — CI-finished notification with dedup

- [ ] A watched PR whose checks reach a terminal state publishes exactly one
      `GithubCiFinished` event carrying workspace id, PR number, head branch,
      title, failed count, and total count.
- [ ] A pending first observation publishes nothing.
- [ ] Dedup across simulated reconnect: after a terminal announcement, a
      re-`watch()` from a stale rollup (list still says in-progress) followed
      by another all-complete poll does **not** publish a second
      `GithubCiFinished` for the same terminal check set.
- [ ] A genuinely new run (different terminal check set for the same PR)
      announces again.
- [ ] `GithubChecksChanged` keeps its existing behaviour (refetch hint on any
      change; first-poll completion still published).

### `worktree_coordinator` — PR checkout into a task worktree

Uses the same cloned-repository fixture style as the existing git tests
(`origin` bare repo + clone), with a PR head branch pushed to origin.

- [ ] Checkout creates a task worktree on the PR head branch under the
      worktrees namespace (`<namespace>/github/…`), registers a new workspace
      row pointing at it (a new workspace node — never a mutation of the
      source workspace), and returns `reused: false`.
- [ ] Repeating the checkout returns the same workspace/path with
      `reused: true` and does not create a duplicate worktree or workspace row.
- [ ] Isolation: a dirty file in the source workspace's worktree is untouched
      by checkout, and the source workspace row still points at its own path.
- [ ] A head branch that does not exist on the remote surfaces an error and
      creates nothing.

### Protocol

- [ ] `github/github_checkout` registered (methods triple, params with
      `deny_unknown_fields`, result mirror, dispatch arm, Tauri command);
      artifacts regenerated — `bridge-protocol` drift tests stay green.
- [ ] `github/ci_finished` notification registered as Transient; payload
      round-trips camelCase.

## Frontend — vitest

### `githubSurface.ts` (pure helpers, node env)

- [ ] `rollupState(pr)` classifies failing / running / passing / none.
- [ ] `ciNotificationText(payload)` renders "CI failed on <branch> — N check(s)"
      and "CI passed on <branch>".
- [ ] `jumpFallbackHint(workspaceBranch, headBranch)` yields no hint when the
      branches match and a "not checked out here" hint when they differ.

### `GitHubPane` (jsdom)

- [ ] Renders the PR list from fixture data with review + rollup chips; opens
      the detail on click; remote HTML stays inert text.
- [ ] Availability states render explicitly: not installed, not authenticated
      (with remediation), no open PRs, and load errors show the message
      instead of a blank pane.
- [ ] A review thread with path+line exposes a jump affordance; clicking it
      calls the jump callback with (path, line, headBranch).
- [ ] "Check out" runs behind a native confirmation naming the PR and branch;
      confirming calls `githubCheckout`; the result (fresh or reused) is
      narrated in the pane; declining calls nothing.
- [ ] A `github/ci_finished` event for the shown workspace refetches the list.

### App-level glue (jsdom)

- [ ] A CI-finished notification renders a toast; clicking it routes to the
      PR: the github dock pane opens with that PR selected. Dismissing removes
      it. Duplicate payloads (same workspace/number/terminal set) do not stack
      duplicate toasts.
- [ ] Jump-to-diff: when the active workspace's branch equals the PR head
      branch the code pane reveal is invoked with path+line and no hint shows;
      when it differs the reveal still happens and the fallback hint renders.
      Nothing crashes when the file is absent (CodePanel's own error state).

## Mock mode (`bun run dev`)

- [ ] All flows reachable without a daemon: PR list/detail, checkout
      (first call fresh, second reused), and a simulated CI completion that
      exercises the toast → deep-link path.

## Manual pass (recorded in `testing/feat-github-surface.md` before #339 closes)

Full loop: open PR list → watch CI fail → jump to the flagged line → fix in a
task worktree checked out from the PR → re-run checks → merge from Bridge.
