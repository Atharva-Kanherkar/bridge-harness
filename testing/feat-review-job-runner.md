# feat/review-job-runner: Test Contract

Sub-issue 1 (#1116) of the standalone PR reviewer epic (#1115). The epic and #1116 are the
authority; this file names what this branch must prove.

## Scope decisions locked before code

- **Method names follow the registry rule.** Every wire method is `domain/<tauri command>` and
  the shell crate asserts `generate_handler![...]` matches the registry 1:1, so the methods are
  `reviews/start_review`, `reviews/list_reviews`, `reviews/get_review`,
  `reviews/cancel_review` and `reviews/retry_review`, not `reviews/start` and so on. The
  notification is `reviews/job_updated` as written.
- **Step implementations are a trait.** The runner drives `ReviewSteps { prepare, review,
  find_posted, post, cleanup }`. Sub-issues 2 to 4 implement it. Until they land, production
  uses steps that fail a job with "This build cannot run PR reviews yet", and nothing in the
  UI calls `reviews/start_review`.
- **`github/github_review` is not flipped in this branch.** Mapping it to `reviews::start` now
  would make the pane's Review button create jobs that cannot run. The flip lands with the
  first PR that makes the pipeline usable end to end. Until then `github_review`, its Bugbot
  branch and its tests stay byte-for-byte the same. `GithubReviewParams.session_id` is
  documented as deprecated.
- **Runner settings live in `ReviewerSettings` now.** `defaultHarnesses`, `fallbackHarnesses`,
  `maxConcurrentReviews` (default 2, range 1 to 4) and `postMode` (`draft` by default) are
  added with serde defaults. A stored payload without them loads unchanged. UI for them is
  sub-issue 5.
- **One migration for the whole epic.** Migration 89 creates all three tables, including the
  columns later sub-issues name (`summary`, `denied_requests`, `outside_diff`, `decided_at`,
  `fixed_in_sha`), so no later sub-issue needs a schema change for those.
- Protocol minor bumps to 1.50. A 1.49 client still pairs; it never calls `reviews/*`.

## Functional Behavior

### Store (migration 89)

- `review_jobs`, `review_passes`, `review_findings` exist with the epic's columns plus
  `review_jobs.{harnesses, dedupe_key, summary, retry_of, attempts, lease_owner,
  lease_expires_at}`.
- Indexes: `review_jobs(workspace_id, pr_number, created_at)`, `review_jobs(status,
  created_at)`, and a partial unique index on `dedupe_key` while `status` is non-terminal.
- `dedupe_key` is `workspace_id|pr_number|head_sha|sorted harness set`, so harness order does
  not matter.

### Job states (`reviews/state.rs`)

- Statuses: `queued, preparing, reviewing, drafted, posting, posted, failed, cancelled,
  interrupted`. Terminal: `posted, failed, cancelled, interrupted`.
- Legal: `queued -> preparing`; `preparing -> reviewing`; `reviewing -> drafted`;
  `drafted -> posting`; `posting -> posted`; lease reclaim `preparing -> queued` and
  `reviewing -> queued`; a failed post the user can retry `posting -> drafted`; and
  `failed`, `cancelled`, `interrupted` from every non-terminal status.
- Everything else is refused with an error, including any exit from a terminal status and
  skipping straight to `posted`.

### `reviews::start`

- Inputs: `workspaceId`, `number`, optional `harnesses`, optional `trigger` (`manual` by
  default). `harnesses` defaults to `defaultHarnesses`, then to the first available of
  `claude`, `codex`, `opencode`. Aliases are canonicalized; Bugbot, unknown harnesses, an
  empty list and more than two harnesses are refused.
- The PR head is resolved before insert (fresh, uncached `gh pr view --json
  headRefOid,baseRefOid,...`), so `head_sha` is the PR's real head.
- Idempotent: the same `(workspace, number, head_sha, harness set)` while a job is
  non-terminal returns that job. After it reaches a terminal status, a new job is created.
- Creates no `sessions` row and touches no existing chat.
- Publishes `reviews/job_updated`.

### Runner (`reviews/runner.rs`)

- Started at boot in both hosts beside the other maintenance loops.
- Startup recovery first: every `preparing | reviewing | posting` job becomes `interrupted`
  with `error = "Bridge restarted during the review"`, and `cleanup` is called for it.
- Claims FIFO by `created_at`: a `queued` job moves to `preparing`, and a `posting` job with no
  lease is claimed as-is (the shape `reviews/post` will use in sub-issue 4). The claim is one
  compare-and-swap, so two runners never both win.
- Slot pool sized by `maxConcurrentReviews`: with the default, two jobs run at once and a
  third waits in `queued`.
- Lease of 45 s, renewed every 15 s. A sweep on every tick puts expired `preparing |
  reviewing` jobs back to `queued` and clears the lease on expired `posting` jobs. A job
  reclaimed after its third claim fails with a reason instead. A runner that stops renewing
  loses the job to another runner's claim within 60 s.
- Transient step errors are retried 3 times with backoff, then the job fails with the last
  error.
- A harness failure on a pass marks that pass `failed` and starts a new pass on the next
  harness in `fallbackHarnesses` not yet tried. The job's `harnesses` reports the harness
  that actually ran. When none is left, the job fails.
- Draft mode stops at `drafted` and releases the lease. Auto mode goes `drafted -> posting ->
  posted` in the same claim.
- Before every post, `find_posted` looks for `<!-- bridge-review:<job id> -->` on the PR's
  reviews. When it finds one, the job is marked posted with that review and nothing new is
  posted.
- Cancellation is cooperative: `reviews::cancel` writes `cancelled`, then trips the job's
  in-process cancel token. Steps receive the token, so a running step stops and `cleanup`
  runs within 5 s.
- Every transition and every pass change publishes `reviews/job_updated`.
- `cleanup(job)` is called on every terminal transition, whether the job posted, failed, was
  cancelled or was interrupted. It is also called once the review step ends, because posting
  reads the stored findings and never the worktree. `cleanup` is idempotent.
- A post a person asked for (draft mode, `posting` with no lease) that fails goes back to
  `drafted` with the error, so they can post again. In auto mode a failed post fails the job.
- A step that panics fails the job with the panic message and frees its slot.

### Protocol

- `reviews/start_review { workspaceId, number, harnesses?, trigger? } -> { job }`
- `reviews/list_reviews { workspaceId, number? } -> { jobs }`, newest first, capped at 50.
- `reviews/get_review { jobId } -> { job, passes, findings }`
- `reviews/cancel_review { jobId } -> { job }`: refused for a terminal job.
- `reviews/retry_review { jobId } -> { job }`: allowed for `failed | interrupted | cancelled`.
  Creates a new job with the same workspace, PR, head SHA, harnesses, trigger and post mode,
  linked by `retryOf`. Idempotent like start.
- Notification `reviews/job_updated { job }`, transient.
- `ReviewJob { id, workspaceId, number, headSha, status, trigger, postMode, harnesses,
  findingCounts { total, critical, major, minor, nit }, error?, postedReviewUrl?, retryOf?,
  createdAt, startedAt?, finishedAt? }`.

## Unit Tests

`bridge-core` `reviews::state::tests`:
- `every_legal_transition_is_accepted`
- `every_other_transition_is_refused` (the full 9 by 9 table)
- `terminal_statuses_have_no_exits`
- `failed_cancelled_and_interrupted_are_reachable_from_every_non_terminal_status`

`reviews::store::tests`:
- `migration_creates_the_review_tables_and_indexes`
- `start_is_idempotent_for_the_same_pr_sha_and_harness_set` (order-insensitive)
- `a_new_job_is_created_once_the_previous_one_is_terminal`
- `a_different_harness_set_is_a_different_job`
- `claims_are_fifo`
- `two_runners_racing_for_one_job_produce_one_claim` (two connections on one file, threads)
- `renewal_fails_once_the_job_is_cancelled_or_reclaimed`
- `an_expired_lease_returns_the_job_to_the_queue`
- `an_expired_posting_lease_keeps_the_job_posting_without_a_lease`
- `the_third_reclaim_fails_the_job`
- `startup_recovery_interrupts_in_flight_jobs_only`
- `list_is_newest_first_and_capped_at_fifty`
- `retry_copies_the_inputs_and_links_retry_of`
- `retry_refuses_an_active_job`
- `cancel_settles_a_queued_job_and_refuses_a_terminal_one`

`reviews::runner::tests` (scripted `ReviewSteps`, injected clock for leases):
- `two_jobs_run_at_once_and_a_third_waits_in_queued`
- `max_concurrent_reviews_sizes_the_pool`
- `cancel_during_reviewing_stops_the_step_and_cleans_up_within_five_seconds`
- `a_harness_failure_falls_back_to_the_next_configured_harness`
- `a_job_fails_when_every_fallback_harness_fails`
- `transient_errors_retry_three_times_then_fail`
- `a_transient_error_then_success_completes`
- `draft_mode_stops_at_drafted_and_releases_the_lease`
- `auto_mode_goes_from_reviewing_to_posted`
- `a_post_retried_after_a_timeout_never_creates_a_second_review`
- `a_runner_that_stops_renewing_loses_the_job_within_sixty_seconds`
- `restart_interrupts_the_job_and_retry_completes`
- `every_transition_publishes_job_updated`
- `cleanup_runs_on_every_terminal_transition`
- `starting_a_review_creates_no_session`
- `a_failed_post_in_draft_mode_returns_to_the_draft`
- `a_panicking_step_fails_the_job_and_frees_its_slot`
- `start_resolves_harnesses_from_the_request_then_the_settings`

`reviewer_settings::tests`:
- `runner_settings_default_and_validate` (concurrency 1 to 4, harness lists canonicalized,
  Bugbot refused in harness lists, post mode defaults to draft)
- `a_payload_stored_before_runner_settings_loads_with_defaults`
- Existing reviewer settings tests stay green unchanged.

`bridge-protocol`:
- The existing registry coverage tests (`TYPED_METHODS` total, wire names unique, domains
  sorted) pass with the new domain.
- `review_job_wire_shape_is_camel_case_and_round_trips`
- `events::tests::review_job_updated_carries_the_wire_job_and_is_reconciled_after_lag`: a
  lagged client gets each job's latest state once, like workflow runs.
- `reviews/job_updated` is pinned transient in `delivery_classes_are_pinned`.

`github_surface::tests`:
- `pr_head_reads_fresh_head_and_base_shas` (fake `gh`).

## Integration / Functional Tests

- `bridged/tests/reviews.rs`, `reviews_methods_round_trip_through_the_daemon`: through the
  real daemon socket with a fake `gh` first on `PATH`. Start creates a queued job and returns
  it. Starting again returns the same id. List, get, cancel and retry round-trip with the
  documented shapes, and `reviews/job_updated` arrives on the connection.
- Shell crate: the registry and `generate_handler![...]` parity test passes.
- `src/api.test.ts`: browser mode lists no reviews and refuses to start one, with an honest
  message instead of fabricating a job.

## Smoke Tests

- `cargo test -p bridge-core reviews` green.
- `cargo test -p bridge-core reviewer_settings` and `api::tests::reviewer` green (old path
  untouched).
- `cargo test -p bridge-protocol` green, and regenerating artifacts leaves no diff.
- `cargo test -p bridged --test reviews` green.
- `bun run build` green (generated TS typechecks against `api.ts`).

## E2E Tests

N/A for this branch. Nothing in the UI starts a review job yet, and the live per-harness E2E
belongs to sub-issues 3 and 4.

## Manual / cURL Tests

- `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin
  generate-protocol-artifacts` produces no diff after commit.
- Open the dev database after a daemon boot:
  `sqlite3 <data dir>/bridge.db ".schema review_jobs"` shows the table and the partial
  unique index.
