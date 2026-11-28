# taste-replay — Test Contract

Sub-issue 3 of the taste layer epic (#1196): #1199. Before the taste layer
hides anything by default, its calls are scored against reviews a person
already triaged. This adds the replay, a reference judge, a threshold sweep
and a synthetic history to tune against. Nothing is written to the ledger.

## Functional Behavior

1. **Forward in time.** Reviews are walked in the order they started. Before
   each one, a judge sees only its repository's signals written strictly
   before it, one per subject and source, as each stood then: a decision
   changed later counts as it was.
2. **Judges.** `Judge::judge(past, candidate) -> Show | Hide { reason }`.
   `NeverHide` is the baseline. `RepeatedDrops { min_drops, min_prs }` (3, 2 by
   default) hides a finding whose normalized title was dropped that often
   across that many PRs, with nothing like it kept, edited, fixed or resolved
   since the last drop, and never a critical or security finding. Suppression
   (#1200) replaces its exact-title match.
3. **Metrics.**
   - Keep rate (kept plus edited, over decided) overall and of what stays
     visible.
   - Every wrongly hidden finding (hidden but kept, edited or fixed), listed.
   - Critical findings hidden, which must be 0.
   - Address rate (posted findings later fixed).
   - Reviews affected.
4. **Sweep.** One row per `(min_drops, min_prs)` pair with hidden, wrongly
   hidden, critical hidden, visible keep rate and reviews affected.
5. **Report.** `render` prints counts and titles, never finding bodies.
6. **Fixture.** `taste::replay::fixture::build` writes a synthetic history
   through the review store at fixed instants. In `octo/repo`, PRs 101 to 105
   have:
   - a nit dropped every time;
   - a critical finding dropped three times, then kept;
   - a near-duplicate of the nit, kept once;
   - an edited finding;
   - a bug that was kept, posted and fixed by the next commit.

   `octo/other` has the same nit, kept.
7. **By hand.** `BRIDGE_TASTE_REPLAY_DB=<copy>` replays a real ledger opened
   read-only.

## Unit Tests

**Rust (`bridge-core/src/taste/replay_tests.rs`, new)**
- `the_fixture_hides_the_nit_once_it_was_dropped_three_times_and_nothing_else`
- `every_metric_on_the_fixture`
- `every_wanted_finding_hidden_is_listed_and_the_report_never_prints_a_body`
- `a_judge_never_sees_the_future` (rewriting every signal from a cutoff on
  leaves every judgment before it unchanged)
- `a_decision_changed_later_counts_as_it_stood`
- `a_sweep_reports_one_row_per_threshold_pair`
- `replay_a_real_ledger` (ignored, by hand)

## Verification
`cargo test -p bridge-core --lib taste::` green.
