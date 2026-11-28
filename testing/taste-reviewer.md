# taste-reviewer — Test Contract

Sub-issue 6 of the taste layer epic (#1196): #1202. Every review reads the
person's approved taste rules for the repository and follows them, and a
finding a rule shaped says which.

## Functional Behavior

1. **Selection.** When a review is prepared, the repository's active taste
   rules and those for every repository are candidates.
   - Only rules with no paths, or with a glob matching a changed file, are
     kept.
   - They are ranked path-specific first, then by outcome score, then by the
     most recent change.
   - At most 12 rules and 8000 characters of rule text are given.
   - Waiting, rejected and other repositories' rules are never given.
2. **Context.** The rules go to `taste.json` in the context folder (`id`,
   `kind`, `rule`, `pathGlobs`), which is copied into the checkout with the
   other context files. `pr.json` carries `tasteRules`, the count. When the
   setting `followTasteRules` (on by default) is off, or nothing fits, there
   is no file and the count is 0.
3. **Prompt.** With rules, the per-PR prompt lists `taste.json` with the other
   context files and says:
   - do not raise what a `review_avoid` rule says not to, unless the change
     makes it critical;
   - raise what a `review_prefer` rule asks for, worded its way;
   - report code that breaks a `code_prefer` rule;
   - add `ruleIds` to findings a rule shaped.

   Without rules the prompt is byte-identical to before, and the stable
   instructions never change.
4. **Ids.** The contract reads `ruleIds` leniently (anything not a list of
   strings is none). After validation, a finding keeps only ids its pass was
   given, once each. The pass records every rule it was given, cited or not.
5. **Store and wire.** Schema 110 adds `review_findings.rule_ids` and
   `review_passes.taste_rule_ids`; `ReviewFinding.ruleIds` and
   `ReviewPass.tasteRuleIds` (omitted when empty); `ReviewJobSettings
   .followTasteRules`.
6. **Draft.** A finding a rule shaped shows "Shaped by your rule: …" for each
   cited rule the person still has.
7. **Settings.** "Follow your taste rules" in Review jobs says what it does in
   both states.

## Unit Tests

**Rust (`bridge-core/src/taste/review_rules_tests.rs`, new)**
- `a_review_gets_its_repositorys_rules_that_fit_its_paths_the_specific_first`
- `at_most_twelve_rules_and_eight_thousand_characters_go_to_a_review`
- `the_context_file_names_each_rule_and_a_pass_reads_back_the_ids_it_was_given`
- `the_prompt_names_the_rules_only_when_there_are_some`
- `a_findings_rules_and_a_passs_rules_are_stored_and_read_back`

**Rust (`reviews/session_tests.rs`)**
- `a_pass_with_taste_rules_names_them_and_keeps_only_the_ids_it_was_given`
- `live_review_follows_taste_rules` (ignored; by hand): on the second-opinion
  fixture, the off-by-one is found with and without a rule against naming
  nits, and with the rule `Sum_All`'s name is not raised.

**Vitest**
- `ReviewDraft.test.tsx`: names the taste rules that shaped a finding, and
  only rules you have.
- `ReviewSettings.test.tsx`: follows your taste rules by default, and says so,
  until turned off.

## Verification
`cargo test -p bridge-core --lib` for `taste::` and `reviews::`; `cargo test
-p bridge-protocol`; Vitest for the reviews components; the live eval on
Claude and Codex.
