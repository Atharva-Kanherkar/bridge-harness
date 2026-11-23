# feat/review-pr-worktree: Test Contract

Sub-issue 2 (#1117) of the standalone PR reviewer epic (#1115). The epic and #1117 are the
authority; this file names what this branch must prove. It builds on the runner merged in
#1124 (`ReviewSteps`, claim tokens, `set_head`, cleanup on terminal transitions).

## Scope decisions locked before code

- **Worktree path per job attempt:** `<worktrees>/reviews/pr-<n>-<job12>-<attempt>`, where
  `job12` is the first 12 hex digits of the job id. The issue's `pr-<n>-<sha12>` would collide
  when two jobs (different harness sets) or two attempts of one job review the same head, and a
  superseded driver's guard would then delete the replacement's checkout.
- **`review_jobs.worktree_path` (migration 90)** records the current attempt's checkout. The
  reaper and the inventory's liveness check map paths to jobs through it, not by parsing names.
- **Job folder:** `<data dir>/review-jobs/<job id>/`, with `context/` inside. It is outside the
  worktree namespace, so the worktree inventory never mistakes it for a checkout.
- **Steps reach the core through a weak reference.** `LiveSteps` implements `prepare`,
  `cleanup`, `has_capacity` and `reap`; `review`, `find_posted` and `post` still fail with "This
  build cannot run PR reviews yet" until #1118 and #1119. Nothing in the UI starts a job yet.
- **Budget gating happens before the claim:** when the review worktree budget is full after a
  sweep, the runner does not claim queued jobs (posting jobs need no worktree and are still
  claimed), so the job stays `queued`, as the issue asks.
- **`maxReviewWorktrees`** (default 8, range 1 to 32) is added to the open `ReviewJobSettings`.
- **The fetch remote** is the remote whose URL is the PR's repository. When none matches (a local
  mirror, an unusual URL) it falls back to the branch's push/upstream remote, then `origin`.
- **Git timeouts:** fetch gets 5 minutes; other git calls keep the 30 s default.
- **`prior_findings.json`** belongs to sub-issue 7 and is not written here.

## Functional Behavior

### Prepare

- Reads the workspace path and the PR (`pr_detail`: title, body, author, base ref, labels,
  changed file count, URL; `pr_review_threads`).
- Fetches `+refs/pull/<n>/head:refs/bridge/reviews/pr-<n>` and the base branch into
  `refs/bridge/reviews/base-<n>` from the PR's remote. A fork PR works because `pull/<n>/head`
  lives on the base repository.
- When the fetched head differs from `job.head_sha`, `Prepared.head_sha` carries the new head
  and the runner moves the job to it (`set_head`) before reviewing.
- `git worktree add --detach <path> <head>`. Creates no branch, no `workspaces` row and no
  sidebar node. The local checkout's HEAD, branch and status are untouched.
- Records `worktree_path` on the job (under the claim) and registers the checkout in the
  worktree inventory with kind `review`.
- Writes `<job dir>/context/`:
  - `pr.json`: repo owner and name, number, title, body, author, url, labels, base ref, base
    SHA, merge base, head SHA, head ref, draft flag, changed file count, `diffTruncated`,
    `omittedFiles`, `diffBytes`.
  - `diff.patch`: `git diff -M <merge-base> <head>`, whole files only, capped at 400 KB. Files
    that do not fit are left out whole and listed in `omittedFiles`.
  - `files.json`: every changed path with `additions`, `deletions` (null for binary), `status`
    (`added | modified | deleted | renamed | copied | typechange`), and `previousPath` for
    renames.
  - `threads.json`: the existing review threads, as `pr_review_threads` returns them.
- Fetch and `gh` failures are transient (retried by the runner). A missing workspace, a
  workspace that is not a git repository, or a PR head that cannot be found is fatal.

### Cleanup, four layers

1. A `ReviewWorktree` guard held in `Prepared`. Its `Drop` runs `git worktree remove --force`,
   then `git worktree prune`, then deletes the job folder, so success, error, cancel and panic
   all clean up.
2. `cleanup(job)` (called by the runner after the review step and on every terminal transition)
   removes every checkout of that job (`pr-*-<job12>-*`) and its job folder, and marks the
   inventory rows removed. It is idempotent.
3. A reaper at startup (after interrupting in-flight jobs), every 10 minutes, and whenever the
   budget is full: removes every directory under `<worktrees>/reviews/` that is not the
   `worktree_path` of a non-terminal job, removes job folders of jobs that are not
   non-terminal, and runs `git worktree prune` on every repository that had review worktrees,
   so `git worktree list` keeps no stale review entry.
4. A budget of `maxReviewWorktrees` checkouts. Review worktrees in use are counted as the
   directories under `reviews/` plus claimed jobs that have none yet. When the count reaches
   the budget after a sweep, queued jobs stay `queued`.

### Inventory

- Kind `review` lives under `reviews/` at depth 1. A review checkout whose job is non-terminal
  is retained ("review <id> is running in it"). Any other review checkout is reclaimable once
  idle for an hour, so the storage UI can show and clear it.

## Unit Tests

`git::tests`:
- `fetching_a_pull_ref_returns_its_head_and_never_touches_the_checkout`
- `a_detached_worktree_has_no_branch_and_force_removal_unregisters_it`

`reviews::prepare::tests` (temp repo plus a local bare "GitHub" remote and the fake `gh`):
- `prepare_checks_out_the_pr_head_with_a_file_the_local_branch_lacks` (worktree has the file;
  local HEAD, branch and `git status` unchanged; no `workspaces` row added)
- `a_fork_pr_prepares_from_its_pull_ref` (head commit on no branch of the base repo)
- `a_moved_head_is_reported_for_the_runner_to_adopt`
- `context_files_describe_the_pr` (`pr.json`, `files.json` with statuses, binary and rename,
  `threads.json` from the `gh` stub)
- `an_oversized_diff_keeps_whole_files_and_lists_the_rest`
- `fetch_failures_are_transient_and_a_missing_repository_is_fatal`
- `cleanup_is_idempotent_and_removes_every_attempt`
- `the_guard_cleans_up_when_a_step_panics` (`catch_unwind`)
- `the_reaper_removes_orphans_and_keeps_live_checkouts`
- `the_budget_refuses_when_full_after_a_sweep`
- `a_killed_process_leaves_nothing_after_restart` (a child process creates a review worktree and
  is SIGKILLed; recovery in a fresh core removes it and `git worktree list` has no entry)

`reviews::runner::tests`:
- `a_full_worktree_budget_holds_queued_jobs_until_a_slot_frees`
- `posting_jobs_are_claimed_even_when_the_budget_is_full`
- `recovery_reaps_review_worktrees`

`reviews::settings::tests`:
- `max_review_worktrees_defaults_to_eight_and_is_bounded`

`worktree_registry::tests`:
- `a_live_review_checkout_is_retained_and_an_idle_one_is_reclaimable`

## Integration / Functional Tests

- Real `LiveSteps` with a scripted review step, through every ending, split into three tests to
  keep each readable: `worktrees_are_gone_after_posting_drafting_and_failing_at_review` (posted
  in auto mode, drafted, failed at review), `worktrees_are_gone_after_a_failed_prepare_and_a_panicking_review`
  and `worktrees_are_gone_after_a_cancel_mid_review`. After each, no directory remains under
  `reviews/`, no job folder remains, and `git worktree list` has only the main checkout.
- `bridged/tests/reviews.rs` runs fully offline: the workspace's `origin` is a local bare
  repository holding `refs/pull/103/head`, and `gh` resolves the repository through its default.
  A retried job prepares a real worktree, fails at the review step with "This build cannot run
  PR reviews yet", and leaves no review worktree behind.

## Smoke Tests

- `cargo test -p bridge-core --lib reviews`, `git::tests`, `worktree_registry`, `store::tests`
  green.
- `cargo test -p bridged --test reviews` green.
- `cargo test -p bridge-protocol` green, and regenerating artifacts leaves no diff.
- `bun run build` green.

## E2E Tests

N/A for this branch: nothing in the UI starts a review job, and the reviewer itself is #1118.

## Manual / cURL Tests

- After the daemon test, `git -C <repo> worktree list` shows only the main checkout and
  `<data dir>/worktrees/reviews/` is empty.
