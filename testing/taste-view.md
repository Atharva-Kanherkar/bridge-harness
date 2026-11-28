# taste-view — Test Contract

Sub-issue 9 of the taste layer epic (#1196): #1205, first part. Learned rules
(#1201) wait for the person's approval, and that needs somewhere to see them.
This adds the Taste tab to the Memory app with the methods behind it. It
lands ahead of #1202, since the reviewer can follow only rules a person
approved.

## Functional Behavior

1. **`taste/list_taste_rules`** `{status?}`: every `taste` record in `active`
   (default), `proposed`, `rejected` or `expired`, across repositories, newest
   change first, at most 500. Any other status is refused.
2. **`taste/get_taste_evidence`** `{recordId}`: the decisions linked to a rule,
   oldest first (verdict, title, path, PR, harness, weight), and `swept`, the
   links whose decision the log no longer keeps.
3. **`taste/add_taste_rule`** `{scopeKey, kind, body, pathGlobs?}`: a rule the
   person writes, active at once, `user_explicit`.
   - The scope is `project:<owner>/<repo>` or `account:local`; the kind is
     `review_avoid`, `review_prefer` or `code_prefer`.
   - Blank globs are dropped.
   - The same text again in the same scope answers with the rule already
     there.
4. **Approve, reject and delete** go through the memory methods
   (`memory/review_memory_records` with the rule's scope,
   `memory/delete_memory_record`).
5. **The Taste tab** (Memory app, between Review and Activity):
   - The tab label counts the rules waiting.
   - "Waiting for you": each rule's kind, text, repository, rationale and
     "From N decisions", with Approve and Reject.
   - "Active rules", grouped by repository, every repository last: each rule
     is "Learned" or "Written by you", shows its paths, and can be deleted.
   - "From N decisions" opens the decisions behind a rule.
   - "New rule": kind, text, repository (empty: every repository) and paths.
     The repository and kind stay for the next rule.
6. **Not in this part:** pause, pin, scores and "never used" (#1203); "Shaped
   by" chips in review drafts (#1202); "Never flag this here" on a dropped
   finding.

## Unit Tests

**Rust (`bridge-core/src/taste/learn_tests.rs`)**
- `the_taste_view_lists_rules_across_repositories_with_the_decisions_behind_them`
- `a_rule_you_write_is_active_at_once_and_written_once`

**Rust (shell)**: `the_protocol_contract_matches_the_registered_command_surface`.

**Vitest (`src/components/TasteRules.test.tsx`, new)**
- lists learned rules waiting for you, with where they came from, and
  approves one
- groups active rules by repository, every repository last, and opens the
  decisions behind one
- writes a rule for a repository or for every repository, and deletes one

## Verification
`cargo test -p bridge-protocol`, `cargo test -p bridged`, `cargo test -p
bridge-core --lib taste::`, the shell's contract test, Vitest for
`TasteRules` and `MemoryDialog`, `tsc -b`.
