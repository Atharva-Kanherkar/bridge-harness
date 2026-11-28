# taste-suppress — Test Contract

Sub-issue 4 of the taste layer epic (#1196): #1200. A new review finding that
resembles ones the person kept dropping arrives hidden in the draft, with the
reason in their words, one click from coming back. This is the first time the
taste layer changes what a person sees.

## Functional Behavior

1. **Alike.** Two findings are alike when they have the same category and the
   same language (from the path), and either their titles overlap by at least
   0.6 (meaningful words, Jaccard) or their titles overlap by at least 0.3 and
   the code they quote in backticks overlaps by at least 0.5. Second opinion's
   looser match (word overlap 0.3, lines nearby) is not used: across pull
   requests there are no lines to lean on.
2. **Hide when**, among the repository's live signals alike to the finding:
   - drops reach 3 (an ignored thread counts half);
   - those drops come from at least 2 PRs;
   - drops outweigh keeps, edits, fixes and resolves (a keep of a finding the
     taste layer had hidden counts twice);
   - nothing alike was wanted after the latest drop.
3. **Never hidden:** critical findings and security findings, whatever was
   dropped.
4. **Where.** When a pass completes, its pending findings are judged against
   the repository's live signals and hidden ones get `hidden_reason` ("You
   dropped N like this, last on #PR.") and `hidden_by_signals`. Best effort: a
   failure is logged and the review goes on. Off when the job settings'
   `hideRepeatDrops` is false (on by default).
5. **Hidden is pending.** Auto-post skips a hidden finding until someone keeps
   or edits it. Deciding a hidden finding logs `{"wasHidden": true}` on its
   signal.
6. **Category from the reviewer.** The output contract asks for `category` (one
   of the nine slugs); validation keeps a known one, case-insensitively, and
   drops anything else without failing the pass. It is stored on the finding
   and preferred over the keyword guess in signals and the replay.
7. **Draft UI.**
   - Hidden findings are listed apart under "Hidden by your taste (n)", each
     with its severity, title, reason and Show again.
   - A finding brought back joins its file's group with a "Your taste hid
     this: …" line.
   - Keep all and Drop all pass over hidden findings.
   - The footer counts them ("4 kept, 1 hidden").
8. **Settings.** "Hide findings you keep dropping" in Review jobs, on by
   default, says what it does in both states.
9. **Replay.** The replay's sweep runs the suppressor.

## Unit Tests

**Rust (`bridge-core/src/taste/suppress_tests.rs`, new)**
- `alike_needs_the_same_kind_language_and_close_titles_or_shared_code`
- `the_fixture_hides_what_was_always_dropped_and_nothing_that_was_wanted`
- `wording_that_changes_between_reviews_is_still_recognised`
- `the_floor_needs_three_drops_on_two_prs_and_never_hides_critical_or_security`
- `a_keep_since_the_last_drop_wins_and_a_kept_hidden_one_counts_double`
- `an_ignored_thread_counts_half_a_drop`
- `a_new_review_drafts_with_the_repeat_hidden_and_the_reason_in_your_words`
- `turning_the_setting_off_hides_nothing`
- `a_sweep_of_suppression_thresholds_on_the_fixture`

**Rust (`reviews/contract_tests.rs`)**
- `a_finding_may_say_what_it_is_about_and_anything_else_is_dropped`

**Vitest**
- `ReviewDraft.test.tsx`: lists what your taste hid apart, with the reason,
  and brings one back on Show again.
- `ReviewSettings.test.tsx`: hides findings you keep dropping by default, and
  says so, until turned off.

## Verification
`cargo test -p bridge-core --lib` for `taste::` and `reviews::`; `cargo test
-p bridge-protocol`; Vitest for the reviews components; live reviewer evals
(`live_second_opinion_on_a_fixture_pr` on Claude and Codex,
`live_review_on_a_fixture_pr` on OpenCode) with the category in the contract.
