# Test contract: Reviews dashboard

Branch `feat/review-dashboard`, issue #1150. Locked before implementation.

## Behaviour

1. **PR title and author on jobs.** A migration adds `review_jobs.pr_title` and
   `pr_author`. `prepare` stores them from the PR metadata it already reads. `ReviewJob`
   gains optional `repoOwner`, `repoName`, `prTitle`, `prAuthor` (omitted when unknown);
   `ReviewJobDetail` gains `summary` (the named form when there is one: the dashboard is
   local).
2. **`reviews/review_overview`** (`ReviewOverviewParams { days: Option<u32>, workspaceId:
   Option<String> }`, both optional; `days` absent means all time, 1 to 365 otherwise):
   - `jobs`: jobs created in the period, newest first, at most 200, plus every live job
     (queued, preparing, reviewing, drafted, posting) whatever its age.
   - `stats`: reviews in the period; counts by state (`running` = queued, preparing,
     reviewing, posting; `waiting` = drafted; `posted`; `failed` = failed or interrupted;
     `cancelled`); findings by severity; decisions (`kept`, `edited`, `dropped`, `pending`);
     `fixedLater` (findings with `fixed_in_sha`); `agreed` (`agreed_by` 2); `medianSeconds`
     (finished minus started, over jobs that drafted or posted; absent when none); spend in
     micro-USD and tokens in and out over their passes.
   - `harnesses`: one row per harness that ran a pass in the period: passes, failed passes,
     findings, kept (kept or edited), dropped, median pass seconds, spend. A merged finding
     counts for each harness in its `pass_ids`.
   - `findings`: findings of the period's jobs, newest first, at most 300, each with its
     job's id, repository, PR number and title, and the harnesses that found it.
   - `workspaceId` limits everything to that workspace.
3. **UI** (`src/components/reviews/ReviewsScreen.tsx`, app `reviews` in the Apps list):
   - Period tabs (7 days, 30 days, All) and a repository filter.
   - Metric tiles: reviews, findings, kept rate (kept + edited over decided), fixed later,
     median time, spend. A rate with nothing decided shows "–", not 0%.
   - **Needs you** (drafted) and **Running** lists; empty states say what to do.
   - **Reviewers** table from `harnesses`.
   - **Reviews** table with status filter; a row opens a detail panel with the summary,
     passes (harness, status, time, cost, error) and `ReviewDraft` triage.
   - **Findings** table with a text search and severity filter.
   - Live: a `reviews/job_updated` notification refetches the overview (coalesced).
   - Browser preview mock for `reviews/review_overview`.

## Tests

Rust (`reviews/overview_tests.rs`):
- `stats_count_states_findings_and_decisions` over a seeded store.
- `kept_rate_inputs_and_fixed_later` (pending excluded from decided).
- `median_seconds_over_drafted_and_posted_jobs`.
- `the_period_cuts_old_jobs_but_keeps_live_ones`.
- `a_merged_finding_counts_for_both_harnesses`.
- `workspace_filter_limits_everything`.
- `jobs_and_findings_are_capped_and_newest_first`.
- `prepare_stores_the_pr_title_and_author` (prepare tests) and the wire round trip of the new
  optional fields.
- Protocol gates: params strictness, registry, artifacts, the shell parity check.

Frontend (`ReviewsScreen.test.tsx`):
- renders tiles from the overview, "–" for a rate with nothing decided;
- Needs you lists drafted jobs; clicking one opens triage (`ReviewDraft`);
- status filter and findings search narrow the tables;
- a `reviews/job_updated` event refetches;
- empty overview shows the empty states.

E2E (`scripts/e2e-review-dashboard.mjs`): a real `bridged --listen` with reviews seeded in
two repositories; the dashboard lists both, its numbers match, a draft is kept and posted
from the dashboard through the fake `gh`; light and dark screenshots.

Live: the dashboard over the real reviews of PR #1133 from the live QA run, screenshotted.

## Amendment from the live QA run (before the fix)

A drafted job has no `finished_at` (it waits for a person), so "finished minus started"
gave no median for drafts. `medianSeconds` is the time from the job's start to its last
finished pass; a posted job's later post time does not count. The overview tests now seed
jobs the way the store leaves them (no finish time on a draft).
