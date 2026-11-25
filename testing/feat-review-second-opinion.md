# Test contract: second opinion across two harnesses

Branch `feat/review-second-opinion`. Locked before implementation.

## What already exists (from earlier slices)

- `ReviewJobSettings.second_opinion: bool` and the rule that a job with two
  harnesses needs it on and the two must differ (`reviews::resolve_harnesses`).
  The issue's `secondOpinion: { enabled, harnesses }` shape is not adopted: the
  pair is `defaultHarnesses`, which already ships and is already validated.
- `review_findings.agreed_by` and the draft order `agreed_by DESC, severity,
  confidence`. The pane's "Both reviewers" badge and per-finding harness label.
- Prepare runs once per job; cleanup runs once after the review step.

## What this branch adds

1. **Parallel passes.** A job with two harnesses runs its two passes at the same
   time when the runner has a second free slot (`maxConcurrentReviews`). The
   second pass holds that slot until it ends. With no free slot, the passes run
   one after the other. Both passes read the one prepared checkout.
2. **A pass's fallback never takes the other slot's harness,** so two passes of
   one job are always two different harnesses.
3. **The job leaves `reviewing` only when both passes have ended.** Cleanup runs
   after both. If one pass fails after its fallbacks, the job drafts with the
   other's findings and the summary starts by naming the reviewer that failed
   and why. Both failing fails the job as today.
4. **Summary.** With two finished passes, each reviewer's summary is labelled
   with its harness name.
5. **Merge** (`reviews/merge.rs`, pure):
   - Candidates: one finding from each pass, same `path`, line ranges
     (`start_line..=line`) no more than 3 lines apart.
   - Match when fingerprints are equal, or title-word Jaccard >= 0.5 (lowercase
     words, a short stopword list removed).
   - Pairs are matched one to one, best first: equal fingerprint, then higher
     Jaccard, then nearer lines.
   - The merged finding keeps the higher severity and higher confidence; its
     body (and title and location) is the longer body's, unless confidences
     differ by more than 0.2, then the more confident one's. `agreed_by = 2`,
     and it lists both pass ids.
6. **Store.** Migration 95 (main took 94) adds `review_findings.pass_ids` (JSON). The merge is
   written in one transaction under the job's claim after both passes end: the
   kept row is updated, the other deleted. A driver that lost the claim writes
   nothing. Wire: `ReviewFinding.passIds` (empty unless merged).
7. **Posting.** A merged finding's comment footer names both harnesses.
8. **Pane.** A merged finding's meta line names both harnesses.

## Tests

`merge.rs` (unit):
- `equal_fingerprints_merge` across passes.
- `near_lines_and_similar_titles_merge` (ranges 3 apart, Jaccard >= 0.5).
- `four_lines_apart_stay_separate`.
- `low_title_overlap_stays_separate`.
- `other_paths_stay_separate`.
- `findings_of_one_pass_never_merge`.
- `higher_severity_and_confidence_are_kept`.
- `longer_body_wins_unless_confidence_differs`.
- `each_finding_merges_once` (one to one, best pair first).

`runner_tests`:
- `two_passes_run_at_once_when_a_slot_is_free` (both reviews in flight together).
- `two_passes_run_in_turn_with_one_slot`.
- `the_second_pass_holds_a_runner_slot` (another job waits meanwhile).
- `prepare_runs_once_and_cleanup_after_both_passes`.
- `one_failed_reviewer_still_drafts_and_says_so`.
- `a_fallback_never_takes_the_other_slots_harness`.
- `agreed_findings_merge_and_rank_first`.
- `a_lost_claim_merges_nothing`.

`store_tests`:
- `merge_findings_updates_and_deletes_under_the_claim`.
- `pass_ids_round_trip_on_the_wire` (and empty for unmerged findings).

`post_tests`:
- `a_merged_comment_names_both_harnesses`.

Frontend (`ReviewDraft.test.tsx`):
- a merged finding shows both harness names; an unmerged one its own.

Live (ignored, by hand): `live_second_opinion_on_a_fixture_pr`. Two real
harnesses review one fixture with one obvious bug and one style issue, in
parallel, through the real session and the real merge. Passes only if the bug
appears exactly once, with `agreed_by = 2`, ranked first. Saves both raw answers
and the score. Run on several harness pairs, more than once each.

## Amendment after the first live runs (before changing the matcher)

Five live pairs (Claude+Codex twice, Codex+OpenCode GPT-5.5, Claude+OpenCode GLM,
Claude+OpenCode GPT-5.5): every reviewer found the bug, and the issue's rule merged
none of them.
- Titles almost never share words: "Reject indices equal to the slice length" and
  "Fix off-by-one bounds check in get" are one finding at one line (title overlap 0).
  Title overlap reached 0.5 for no true pair.
- Claude gives the last line of the block (`line` 12 for an `if` on line 8) without a
  `start_line`, so its findings sit 4 lines from the others'.
- The bodies agree: they quote the same code (`index == items.len()`, `items[index]`).

Measured over every cross-reviewer pair: true pairs (the bug, and both naming
findings) have quoted-code overlap 0.29 to 0.86 and title+body word overlap 0.21 to
0.42; different findings (bug against naming, "No tests" 3 lines from the bug) have at
most 0.08 and 0.14.

So the rule becomes:
- Candidates: same `path`, ranges no more than **6** lines apart.
- Match when fingerprints are equal, or title-word overlap >= 0.5, or **quoted-code
  overlap** (backticked spans in title and body, whitespace removed) >= 0.25, or
  **title+body word overlap** >= 0.3.
- Best pair first: equal fingerprint, then the highest of the three overlaps, then
  nearer lines.

Tests added: `quoted_code_in_the_bodies_merges` and `four_lines_apart_merge_when_the_bodies_agree`
(the real Codex/GPT-5.5 and Claude/Codex pairs, word for word), `a_test_gap_finding_near_the_bug_stays_separate`
and `naming_and_the_bug_stay_separate` (the real negatives), `seven_lines_apart_stay_separate`
(replaces `four_lines_apart_stay_separate`). The live eval is then rerun on fresh runs the
thresholds were not chosen on.

## Amendment for the review on f0a0bf1b (before the fixes)

- **Same side only.** Two findings are candidates only when they are on the same diff side
  (`LEFT` or `RIGHT`); `LEFT:10` and `RIGHT:10` are different code after a large insertion.
  Test: `other_sides_stay_separate` (the reviewer's case: a 100-line insertion's missing check
  at `RIGHT:10`, a deleted guard at `LEFT:10`, both quoting the guard).
- **A merged finding keeps the identities it absorbed.** Migration 95 adds
  `review_findings.merged_identities` (JSON `[{title, fingerprint}]`); the merge writes the
  dropped finding's title and fingerprint there. `prior_findings.json` carries them as
  `alsoKnownAs`, and a re-review's repeat filter matches a new finding against the settled
  finding's own identity or any of these. Tests: `merge_findings_keeps_the_dropped_identity`
  (store), `a_merged_findings_other_title_is_a_repeat` (session, the live fixture's two
  bounds-check titles), and the prepare writer test.
- **Every runner slot goes through one bounded reservation.** `tick` reserves a slot with the
  same compare-and-swap as a second pass before it claims a job, and gives it back when nothing
  is claimed or the claim fails. Test: `a_second_pass_and_a_queued_job_never_exceed_the_limit`
  (a second pass reserving between tick's check and its claim; `active()` never passes
  `maxConcurrentReviews`).
- **Attribution hiding covers the summary.** The runner stores two summaries: `summary`, with
  no reviewer named ("Reviewer 1", "Reviewer 2", and a failure note that names neither the
  harness nor its error), and `named_summary` (migration 95), with names and the reason.
  Posting uses `named_summary` only when attribution is shown, read at post time. Tests:
  `a_hidden_attribution_post_names_no_reviewer` (post, success and partial failure) and the
  runner's summary tests on both columns.
