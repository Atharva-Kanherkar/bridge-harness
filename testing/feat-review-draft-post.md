# feat/review-draft-post: Test Contract

Sub-issue 4 (#1119) of the standalone PR reviewer epic (#1115). The epic and #1119 are the
authority; this file names what this branch must prove. It builds on the runner (#1124), the
prepared checkout (#1126) and the reviewer session (#1129).

## Scope decisions

- **Method names follow the registry rule** (`domain/<tauri command>`):
  `reviews/decide_review` and `reviews/post_review`, not `reviews/decide` and `reviews/post`.
  Protocol minor goes to 1.52; a 1.51 client still pairs (it never calls them).
- **`postMode` already exists** in `ReviewJobSettings` (draft by default, from #1124). Nothing
  new there.
- **`event` cannot be anything but `COMMENT`.** The payload's `event` field is a unit type
  whose only serialization is `"COMMENT"`. No enum, no parameter, no string field.
- **What posts:** in draft mode, findings decided `kept` or `edited`; `pending` and `dropped`
  never post. In auto mode, every finding not `dropped` (nobody decides). Inline findings become
  comments; `outside_diff` findings go in the review body.
- **Posting is allowed** in `drafted`, on a job no runner holds, when at least one finding is
  kept or edited, or the job has no findings at all (a summary-only review). Findings that are
  all pending or dropped refuse the post with a plain reason.
- **Decisions:** `kept | dropped | edited`, only in `drafted` on a job no runner holds.
  `edited` needs a non-empty `editedBody`; `editedBody` with any other decision is an error.
  `kept` and `dropped` clear an earlier edit. A batch is all or nothing: an unknown finding id,
  a finding of another job, or the same finding twice refuses the whole batch. Each decision
  stores `decided_at`, exposed as `ReviewFinding.decidedAt`.
- **`github_comment_id` and `posted_at`** record what GitHub holds. A fresh post marks every
  posted finding; comment ids come from the review's comments, matched by path, line and side
  (exact body first). A review found by its marker marks only the findings it actually holds
  (a matching comment, or an outside-diff title in its body).
- **A draft-mode post interrupted by a restart returns to `drafted`** with "Bridge restarted
  while posting; post again" instead of becoming `interrupted`, so the triage survives. The
  marker check makes the next post safe. Auto-mode posts still become `interrupted`.
- **Head moved:** the review posts at the reviewed `commit_id`, and its body says the PR has
  moved on to the new head. A failed head read only drops that line.
- **GitHub calls** go through `GithubSurface::run_gh` (same binary, auth and timeouts). The
  payload goes in a private temp file passed with `--input`. Errors: HTTP 422, 401, 403 and 404
  are fatal; timeouts, 5xx, 429 and network failures are transient. The runner already checks
  the marker before every post attempt.
- `github/github_review` is not flipped here (that is #1120, with the pane).

## Functional Behavior

### Payload (`reviews/post.rs`)

- `POST repos/{owner}/{repo}/pulls/{n}/reviews` with
  `{commit_id: head_sha, event: "COMMENT", body, comments: [{path, line, side, start_line?,
  start_side?, body}]}`. `start_line` and `start_side` only for a range (`start_line < line`).
- Comment body: a severity label, the title, the body (the edited body when `edited`), then a
  footer naming the harness only when attribution hiding is off.
- Review body: the job summary (or a plain line when empty), the head-moved line when it applies,
  an "Outside the diff" section, the footer when hiding is off, and
  `<!-- bridge-review:<publication key> -->` last.
- Bodies stay under GitHub's 65,536-character limit: long comment bodies are cut, and
  outside-diff entries that do not fit are counted instead.

### Protocol

- `reviews/decide_review {jobId, decisions: [{findingId, decision, editedBody?}]} ->
  {findings}`.
- `reviews/post_review {jobId} -> {job}`: `drafted -> posting`, clears the last error, wakes
  the runner, publishes `reviews/job_updated`.
- Auto mode: `reviewing -> drafted -> posting -> posted` with no call.
- A failed draft-mode post returns to `drafted` with the reason (from #1124); auto-mode fails.

## Unit Tests

`reviews::post::tests`:
- `every_payload_says_comment_and_nothing_else` (draft, auto, empty, ranges, LEFT,
  outside-diff, footer on and off)
- `no_review_source_names_another_event` (no `APPROVE` or `REQUEST_CHANGES` outside tests)
- `only_kept_and_edited_findings_post_in_draft_mode`
- `auto_mode_posts_everything_not_dropped`
- `ranges_and_left_side_comments_carry_their_fields`
- `outside_diff_findings_fold_into_the_body`
- `the_footer_names_the_harness_only_when_attribution_is_shown`
- `a_moved_head_is_named_in_the_body`
- `bodies_stay_under_the_github_limit`
- `returned_comments_map_back_to_findings`
- `a_found_review_marks_only_what_it_holds`
- `gh_errors_map_to_retry_or_fail`

`reviews::store::tests`:
- `decisions_are_stored_with_a_timestamp_and_survive_reopening`
- `decide_refuses_outside_a_waiting_draft_and_bad_batches`
- `post_requires_a_kept_finding_or_no_findings`
- `marking_posted_records_comment_ids_and_posted_at`
- `a_draft_post_interrupted_by_restart_returns_to_the_draft`

## Integration / Functional Tests

`reviews::post::tests` with a `gh` stub and the real runner:
- `a_draft_post_reaches_github_once_with_the_kept_findings`
- `a_post_that_timed_out_after_github_took_it_is_not_posted_twice`
- `an_auto_job_posts_without_anyone_deciding`
- `a_rejected_post_returns_the_draft_with_the_reason`

`bridged/tests/reviews.rs` (offline): after the job drafts, decide the finding `edited` and post
over the socket; the job ends `posted` with the stub's URL, the stub received exactly one
`COMMENT` payload carrying the edited body inline at the reviewed commit, and the finding has
its `githubCommentId` and `postedAt`.

`src/api.test.ts`: browser mode refuses decide and post instead of inventing a result.

## Live Tests (run by hand, `--ignored`)

- `reviews::post::tests::live_post_on_a_sandbox_pr` with `BRIDGE_REVIEW_LIVE_REPO=owner/name`
  and `BRIDGE_REVIEW_LIVE_PR=n`: posts one `COMMENT` review with a single-line, a multi-line and
  a LEFT comment, checks GitHub placed them on those lines, and checks a second post finds the
  marker instead of posting again. Run only on a sandbox repository, with permission.

## Smoke Tests

- `cargo test -p bridge-core --lib reviews` green.
- `cargo test -p bridged --test reviews` green.
- Protocol artifacts regenerated; `bun run build` and `bun run test src/api.test.ts` green.

## E2E Tests

N/A: the pane gets its UI in #1120.

## Manual

- None beyond the live test.
