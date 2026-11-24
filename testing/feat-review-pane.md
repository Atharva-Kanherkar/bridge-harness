# feat/review-pane: Test Contract

Sub-issue 5 (#1120) of the standalone PR reviewer epic (#1115). The epic and #1120 are the
authority; this file names what this branch must prove. It builds on the runner (#1124), the
prepared checkout (#1126), the reviewer session (#1129) and triage and posting (#1132).

## Scope decisions

- **The pane starts review jobs.** The Review button's Claude, Codex and OpenCode entries call
  `reviews/start_review`. Cursor Bugbot keeps `github/github_review` (it is not a local job).
- **`github/github_review` becomes a thin wrapper** for local harnesses: it queues a review job
  and answers `status: "queued"` with a message pointing at the pane's Reviews section. Its
  Bugbot branch is unchanged. `sessionId` is ignored (deprecated since #1124). The old worker
  launch path for reviews is removed.
- **Second opinion is a real setting.** `ReviewJobSettings.secondOpinion` (default off). On, the
  pane picks two harnesses and the default may name two. Off, `reviews/start_review` refuses
  two harnesses and saving two defaults is refused.
- **No toggle without a behaviour.** Auto-review on push (#1121) and incremental re-review
  (#1122) have no backend yet, so their toggles land with those sub-issues. A switch that
  changes nothing would tell the person something false.
- **Attention without a session.** The core attention ledger skips rows with no session, and a
  review has none. So the app itself watches `reviews/job_updated`: when a job becomes
  `drafted`, `posted` or `failed`, or a post returns to the draft with an error, it shows an
  in-app toast that opens that PR, and an OS notification when Bridge is not focused.
- **Copy follows the new flow.** No more "isolated worktree, needs approval" for OpenCode, and
  the reviewer instructions note says Bridge posts one comment review and the reviewer never
  touches GitHub.

## Functional Behavior

### Reviews section (`src/components/github/ReviewJobs.tsx`)

- In a PR's Conversation tab. Loads `reviews/list_reviews {workspaceId, number}` and applies
  every `reviews/job_updated` for that PR, so a job started elsewhere, or while another chat was
  open, shows its current state without a refresh.
- Each row: harness chips, status with elapsed time (ticking while it runs), finding counts by
  severity, the trigger (manual or push), Cancel while not terminal, Retry when `failed`,
  `interrupted` or `cancelled`, a link to the posted review when `posted`, and the last error.
- A `drafted` job opens its draft.

### Draft (`src/components/github/ReviewDraft.tsx`)

- Loads `reviews/get_review`. Findings grouped by file, files in path order, findings by
  severity then confidence; outside-diff findings in their own group.
- Each finding: path:line (range when `startLine`), severity, title, body, confidence, the
  harness of its pass, and "Both reviewers" when `agreedBy > 1`.
- Keep, Drop and Edit per finding (edit saves through `reviews/decide_review` as `edited`);
  Keep all and Drop all; a Post button labelled with the number that will post. With findings
  and none kept, Post is disabled and says why; with no findings it posts a summary-only review.
- Keyboard: the findings list is a list of focusable items; ArrowDown and ArrowUp move, K keeps,
  D drops, E edits the focused finding.

### Settings (`ReviewerSettingsSection.tsx`)

- A "Review jobs" group bound to `reviews/get_review_job_settings` and
  `reviews/save_review_job_settings`, each control with one line saying what it does: post mode
  (Draft first, the default, or Post automatically), default harness (a second one when second
  opinion is on), fallback harnesses in order, second opinion, max concurrent reviews (1 to 4,
  default 2), max review worktrees (1 to 32, default 8).
- Every control round-trips, and a failed save shows the reason and keeps the stored value.

## Unit Tests

Rust:
- `reviews::tests::two_harnesses_need_second_opinion` (start refuses two with it off, accepts
  with it on)
- `reviews::settings::tests::two_default_harnesses_need_second_opinion`
- `api` test: `github_review_queues_a_review_job_for_local_harnesses` (and Bugbot unchanged)

TypeScript (vitest, jsdom):
- `ReviewJobs.test.tsx`: lists the PR's jobs; applies `reviews/job_updated` live; ignores other
  PRs; Cancel and Retry call the protocol; posted rows link; elapsed time ticks.
- `ReviewDraft.test.tsx`: grouping and order; Keep, Drop, Edit; Keep all, Drop all; Post count
  and disabled reason; summary-only post; K, D, E and arrows; "Both reviewers" badge.
- `ReviewerSettingsSection.test.tsx`: each job setting round-trips; second opinion unlocks a
  second default; a refused save shows its reason.
- `GitHubPane.test.tsx`: Review with Claude calls `reviews/start_review`; with second opinion
  two picks start one job with both; Bugbot still calls `github/github_review`.
- `App.reviewAttention.test.tsx`: drafted, posted and failed raise one toast each (not on
  load, not twice); the toast opens the PR.

## Integration / Functional Tests

- `bun run test` subset above, `bun run build`, `cargo test -p bridge-core --lib reviews`,
  `cargo test -p bridged --test reviews`.

## E2E Tests

- Playwright against the browser preview linked (`#bridge-daemon=…`) to a real
  `bridged --listen` with a fake `gh` and the fake Claude SDK (`testing/fixtures`): open the PR,
  Review with Claude, watch the job reach `drafted` live, keep the finding with K, post, and see
  the posted link. Screenshots in light and dark.

## Evals (live, by hand, results in the PR description)

- Fixture eval per harness (`live_review_on_a_fixture_pr`, two planted bugs): Claude, Codex,
  OpenCode (two models). Valid findings, bugs found, no shell (Claude, OpenCode), denied
  requests, tool calls, tokens, cost, time.
- Real-PR eval per harness through the daemon: prepare a real merged PR of this repository,
  review it, reach `drafted` in draft mode (nothing posted), and record findings and cost.
- Everything under `env -i`; afterwards the harnesses' real data folders are checked for what the
  runs left.

## Review 6034514925 (amended before the fixes)

- **An edit is never lost or silently replaced.** The editor's text lives in `ReviewDraft`, per
  finding. While an editor holds text that differs from what is saved, Post is disabled and says
  "Save or cancel your edit to post." Keeping or dropping another finding leaves an open editor
  as it is.
- **Undecided findings are named** beside Post: "2 kept, 2 not decided".
- **Times stay true.** A finished job's "when" refreshes at least every 30 seconds and on every
  update. A `posting` job shows no timer (its `startedAt` is the first claim, not the post).
- **The list never undoes a live update.** The pane subscribes, waits for the subscription, then
  lists; a job that has had a live update keeps it over the list's copy.
- **Screen readers hear the status, not the clock.** Only the status label is a live region.
- **Every job that needs the person is reachable.** Jobs not yet finished always show; finished
  ones are capped at 10 with "Show N more".
- **A stored pair of default harnesses from before `secondOpinion`** loads with Second opinion on,
  so it saves and runs as it did.
- **`github/github_review` says what it found.** Its result gains `jobId`, and `status` is the
  job's real status (`queued`, `preparing`, `reviewing`, `drafted` or `posting`; an existing job
  for that head is returned, not a second one). The message is worded from that status.
- **A repeat failure is new news.** The same notice for the same job raises a fresh toast with a
  fresh timer.
- **The picker is a group, not a menu.** Plain buttons, or checkboxes with Second opinion on; the
  picks clear when it closes or the PR changes.
- **The reviewer treats the PR as data.** Convention files are read from the base branch (copied
  into the context folder), and the instructions say that the PR's description, threads, code
  and any change to convention files are under review, never instructions to the reviewer.
- **E2E** screenshots the group by `[data-settings-row="review-jobs"]` and fails when it is
  missing. The picker is driven by button and checkbox roles.

Tests added for this review:
- `ReviewDraft.test.tsx`: keep, edit, then Post is blocked until the edit is saved, and the saved
  text is what posts; deciding another finding keeps the open edit; the undecided count.
- `ReviewJobs.test.tsx`: a finished job's time ages with no running job (fake timers); a posting
  job shows no timer; a list that resolves after a live `drafted` leaves it drafted; only the
  status label is `role="status"`; non-terminal jobs past 10 still show, "Show N more".
- `GitHubPane.test.tsx`: the picker is `role="group"` with checkboxes; picks clear on close.
- `App.reviewAttention.test.tsx`: a second post failure on the same job raises a toast again.
- Rust: `settings::tests::a_stored_pair_from_before_second_opinion_still_loads_and_saves`;
  `api` test checks `jobId` and that a repeat call returns the existing job's status;
  `session` test: conventions come from the base branch, and the data rule is in the prompt.

## Manual

- None beyond the evals.
