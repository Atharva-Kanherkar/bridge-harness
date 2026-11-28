# taste-scores — Test Contract

Sub-issue 7 of the taste layer epic (#1196): #1203. Each taste rule earns or
loses trust from what happens next. A learned rule that turns noisy pauses
itself, and one nothing confirms expires. Rules the person wrote or pinned do
neither.

## Functional Behavior

1. **Events**, rebuilt from the review tables and the decision log:
   - a finding that cites a rule: kept or edited `+1`, fixed `+1`, dropped
     `-1`;
   - a posted review whose passes were given a `review_avoid` rule: `+0.5`,
     or `-1` when a kept finding of that review is like the rule's evidence;
   - a hidden finding the person kept: `-2` for each rule whose evidence
     intersects what hid it.
2. **Score** is the sum of events, each halved every 60 days of age, stored to
   two decimals. `last_confirmed_at` is the latest positive event.
3. **Pause.** A learned (model-proposed), unpinned, active rule pauses when
   its score is at most `-2`, or when its events of the last 30 days number at
   least 3 and at least 60% of them are against it. The reason is stored
   ("Paused: its trust fell to …" or "Paused: N of its last M uses went
   against it."). A paused rule reaches no review and no chat.
4. **Expire.** Such a rule with no positive event for 90 days (since creation
   when it has none) is retired to `expired`.
5. **Recompute** runs on the maintenance tick at most once an hour.
6. **Never used.** A rule given to at least 10 passes and cited by none is
   listed in `neverUsed`. `review_avoid` rules are never listed: they shape a
   review by leaving things out.
7. **By hand.** `taste/set_taste_rule_state` `{recordId, state?, pinned?}`:
   - pause an active rule ("Paused by you.");
   - resume a paused one (clears the reason and confirms it now);
   - pin or unpin any rule.

   Other transitions are refused.
8. **Schema 111**: `memory_records.pinned`, `memory_records.paused_reason`.
   Wire: `MemoryRecord.pinned`, `.pausedReason`;
   `ListTasteRulesResult.neverUsed`; `list_taste_rules` accepts `paused`.
9. **Taste tab:**
   - Each learned rule shows "Trust ±n.n" and, when it applies, "Never used".
   - Edit, Pin (learned rules only), Pause and Delete on active rules.
   - A Paused section with each rule's reason and Resume.

## Unit Tests

**Rust (`bridge-core/src/taste/score_tests.rs`, new)**
- `trust_rises_with_what_a_rule_shaped_and_kept_and_halves_every_sixty_days`
- `a_learned_rule_that_keeps_getting_dropped_pauses_itself_and_yours_never_do`
- `mostly_against_lately_is_noisy_even_before_the_score_falls_far`
- `a_learned_rule_nothing_confirms_for_ninety_days_expires`
- `an_avoid_rule_earns_on_posted_reviews_and_loses_when_you_raise_what_it_avoids`
- `keeping_a_finding_the_rules_evidence_hid_costs_it_two`
- `a_rule_given_to_ten_reviews_and_cited_by_none_is_never_used_unless_it_avoids`
- `you_can_pause_resume_and_pin_a_rule`

**Vitest (`TasteRules.test.tsx`)**
- shows trust and rules never used, and pauses, resumes, pins and edits

## Verification
`cargo test -p bridge-core --lib` for `taste::`, `memory` and `reviews::`;
the shell's contract test; Vitest for `TasteRules`; `tsc -b`.
