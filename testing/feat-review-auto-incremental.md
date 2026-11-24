# feat/review-auto-incremental: Test Contract

Sub-issues 6 (#1121, auto-review on push) and 7 (#1122, incremental re-review) of the standalone
PR reviewer epic (#1115), in one branch. The epic and the two issues are the authority; this file
names what this branch must prove. It builds on the runner (#1124), the prepared checkout
(#1126), the reviewer session (#1129), triage and posting (#1132) and the review pane (#1135).

## Scope decisions

- **Three new settings, all off by default, all visible.** `ReviewJobSettings` gains
  `autoReview: "off" | "mine" | "all"` (default `off`), `skipDraftPrs: bool` (default `true`) and
  `incremental: bool` (default `false`). Settings → Background tasks → Review jobs shows each with
  one line saying what it does. The pane's job row says when a job was started by a push and
  when it is a re-review.
- **No new polling of unrelated repositories.** Auto-review watches only repositories whose
  GitHub pane listed pull requests since Bridge started, plus workspaces that already have
  review jobs. One `gh pr list` (number, head, draft, author) per watched repository per minute,
  and only while auto-review is on.
- **The first listing is a baseline.** Turning auto-review on, or starting Bridge, never reviews
  every open PR at once. A PR is reviewed when its head changes or it first appears after the
  baseline. One exception: a PR whose latest review was started by a push and covered an older
  head is reviewed again (it was pushed while Bridge was closed).
- **Debounce.** A change waits 120 seconds since the last change it saw for that PR, then one
  job starts at the newest head (`trigger: push`, default harnesses).
- **Superseding.** Starting a push job cancels queued push jobs of the same PR at other heads
  ("Superseded by a newer push"). Manual jobs are the person's and are left alone.
- **Off means off.** With `autoReview: off` nothing is listed, pending changes are forgotten, and
  settings are read again right before each start, so no job starts after the save returns.
- **Incremental applies to every start** (manual or push) when on: `incremental_from_sha` is the
  head of the PR's latest `posted` job when it differs from the new head.
- **Each re-review is a fresh session.** Memory comes from stored findings, never from resuming
  a transcript.
- **Force-push falls back** to a full review and says so in the summary.
- **Repeats.** A new finding repeats a prior one when it has the same fingerprint, or the same
  path and title. It is left out when that prior one was dropped, posted, or its thread was
  resolved, unless the new one is more severe (the new commits made it worse).

## Functional Behavior

### Auto-review (`reviews/auto.rs`, the GitHub poll maintenance loop)

- `observe` (pure): given the settings, the viewer's login, a listing and the time, updates
  per-PR state and returns nothing to start; `due` returns the PRs whose debounce has passed.
- Filters: `mine` keeps PRs whose author is the signed-in `gh` user (unknown viewer: none);
  `skipDraftPrs` drops drafts. A PR that becomes ready for review counts as a change.
- `tick(core, now)`: for each watched workspace due for a listing, list, observe, then start
  each due PR through `reviews::start` with `trigger: push`.

### Incremental re-review

- `reviews::start_with` sets `incremental_from_sha` as above; retry keeps it.
- `prepare` (when `incremental_from_sha` is set): fetches that commit if it is missing; if it is
  an ancestor of the head writes `incremental.patch` (`git diff <from>..<head>`, same size cap);
  else records the fallback. Writes `prior_findings.json`: every finding of the PR's earlier jobs
  with id, job, head, path, line, severity, title, body (the edited body when edited),
  decision, posted, thread state (resolved, outdated, replies) and `fixedInSha`. `pr.json`
  gains `incremental` (`from`, `to`, `fellBack`).
- The per-PR turn names `incremental.patch` and `prior_findings.json` and the rules (review the
  new commits, the full diff is context; do not repeat dropped, posted or resolved findings
  unless the new commits made them worse; list fixed earlier findings). The stable prefix keeps
  its order and does not vary per PR.
- Output contract: optional `"fixed": ["<prior finding id>"]`.
- After validation: repeats are left out (rule above); `fixed` ids that name a prior finding
  that was kept, edited or posted get `fixed_in_sha` = this head, in the same transaction as
  the pass's findings.
- The job summary starts `Re-review of <from12>..<to12>`, or with the force-push fallback line.
- Wire: `ReviewJob.incrementalFromSha`, `ReviewFinding.fixedInSha` (both optional).

### Pane and settings

- Review jobs group: "Auto-review" (Off / My PRs / All PRs), "Skip draft PRs" (disabled while
  auto-review is off), "Incremental re-review". Each round-trips and a refused save shows why.
- Job row: "On push" (existing) and "Re-review since abc1234".

## Unit Tests

Rust:
- `reviews::auto` tests: a head change starts one job after the debounce; two pushes within a
  minute start exactly one job at the last head; the baseline starts nothing; a PR appearing
  after the baseline is reviewed; the restart catch-up rule; filter matrix (off / mine / all x
  draft / ready x skip on / off, unknown viewer); a closed PR is forgotten; turning off forgets
  pending changes.
- `tick` against the fake `gh` and a real store: exactly one push job at the last head; a
  queued older push job is superseded; a manual job is not; off starts nothing.
- `start_with`: `incremental_from_sha` from the latest posted job only when on and the head
  moved.
- `prepare`: ancestor case writes `incremental.patch` with only the new commit; force-push case
  falls back and writes no patch; `prior_findings.json` carries decisions, posted state and
  thread states (resolved, outdated, replies).
- `session`: dedupe matrix (fingerprint or path+title x dropped / posted / resolved / kept x
  same or higher severity); `fixed` marks only known kept, edited or posted priors; prompt
  snapshot: the stable prefix is first and identical for an incremental and a full review, the
  turn names the incremental files.
- `contract`: `fixed` parses and defaults to empty.
- `runner`: summary lead for incremental and fallback.
- `settings`: new fields default, round-trip, and load from a record without them.

TypeScript (vitest):
- `ReviewerSettingsSection.test.tsx`: auto-review mode, skip drafts, incremental round-trip;
  skip drafts disabled while off.
- `ReviewJobs.test.tsx`: "Re-review since" chip.

## Integration / Functional Tests

- `cargo test -p bridge-core --lib reviews github_poll`, `cargo test -p bridged --test reviews`,
  protocol tests and registry parity; vitest subset; `bun run build`; clippy ratchet on touched
  files.

## E2E Tests

- Playwright against the browser preview linked to a real `bridged --listen` (as in
  `scripts/e2e-review-pane.mjs`): Settings shows the three controls and they persist across a
  reload; a re-review job row shows "Re-review since". Screenshots in light and dark.

## Evals (live, by hand, results in the PR description)

- Incremental fixture eval per harness (Claude, Codex, OpenCode): a PR with a posted review of
  two findings (one kept and posted, one dropped); a new commit fixes the kept one and adds one
  new bug. Expect exactly one new finding, the fixed one listed in `fixed`, no repeat of the
  dropped one. Record time, tokens, cost.
- Everything under `env -i`; the harnesses' real data folders are checked afterwards.

## Manual

- None beyond the evals.
