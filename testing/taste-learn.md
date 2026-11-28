# taste-learn — Test Contract

Sub-issue 5 of the taste layer epic (#1196): #1201. The memory model reads a
repository's review signals and proposes plain-language rules, each linked to
the findings it came from, for the person to approve. Nothing becomes active
without them.

## Functional Behavior

1. **Groups.** A repository's live review signals that no record claims as
   evidence are clustered by the suppressor's likeness, each against its
   cluster's first signal.
   - **Avoid group:** drops (an ignored thread is half a drop) reach 3 and
     outweigh wants.
   - **Prefer group:** keeps, edits, fixes and resolves reach 3 and outweigh
     drops.
   - Either way the group needs signals from at least 2 PRs.
   - Drops of a critical or security finding never count toward an Avoid
     group.
   - Groups are ordered by their earliest signal, then by title.
2. **Grown only.** A group is offered only when one of its signals is newer
   than the repository's last learning run, so one the model passed over is
   not offered again until it grows.
3. **Allowance.** At most 5 learned proposals per repository every 14 days.
   With no group or no allowance, the run is recorded as skipped and no model
   turn is spent.
4. **Prompt.** It holds:
   - the kinds (`review_avoid` for dropped groups; `review_prefer` or
     `code_prefer` for kept ones);
   - the 240-character limit;
   - the repository's existing rules, so none is repeated;
   - each group, with its verdicts, titles, paths, clipped bodies and edits.

   The answer is the last fenced json block, or bare JSON:
   `{"rules":[{"group","kind","rule","pathGlobs","rationale"}]}`.
5. **Gate.** A draft is refused, with the reason recorded, when:
   - its group does not exist;
   - its group already got a rule this run;
   - its kind does not fit the group;
   - it is empty or longer than 240 characters;
   - it cannot be stored (a secret);
   - fewer than half its content words, or fewer than two, appear in the
     evidence. Instruction words ("do not", "ask", "stop", "raise", "never")
     are not counted either way, and a plural matches its singular. The prompt
     asks for the reason in `rationale`, not in the rule.
6. **Repeat.** A draft whose words overlap an existing taste rule of the
   repository by at least 0.6 lends that rule the group's evidence instead of
   becoming a new one.
7. **Write.** A proposal is a `taste` record in the repository's scope:
   `model_proposal`, `proposed`, with an empty validity interval, its kind,
   rationale and path globs, and every group signal linked as evidence.
8. **Runs.** Each run is recorded in `taste_learn_runs` (schema 108) with
   offered, proposed, attached, refused, spend and detail. An unreadable
   answer is a failed run.
9. **Nightly.** On each maintenance tick, if dreaming is on, a memory model
   resolves, and a repository with a grown group was not looked at since
   tonight's dream hour, that one repository is learned with one bounded
   hidden turn.

## Unit Tests

**Rust (`bridge-core/src/taste/learn_tests.rs`, new)**
- `the_fixture_has_one_group_the_nit_dropped_on_five_prs`
- `a_drafted_rule_lands_as_a_taste_proposal_with_its_evidence`
- `drafts_that_do_not_fit_are_refused_and_nothing_is_written`
- `a_repeat_of_a_rule_the_repository_has_lends_it_the_evidence`
- `at_most_five_learned_proposals_a_fortnight`
- `findings_kept_again_and_again_teach_how_code_should_be_written`
- `a_group_the_model_passed_over_is_offered_again_only_once_it_grows`
- `an_answer_is_read_from_its_last_json_block_or_as_bare_json`
- `grounded_rules_use_the_evidence_words`
- `a_repository_is_due_once_a_night_while_dreaming_is_on`
- `live_learn_on_the_fixture` (ignored; by hand on Claude and Codex)

## Verification
`cargo test -p bridge-core --lib taste::` green; the live eval on Claude and
Codex proposes a grounded avoid rule for the dropped nit and a prefer rule
for the kept `?` advice, and no rule about the dropped SQL injection.
