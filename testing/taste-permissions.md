# taste-permissions — Test Contract

Sub-issue 11 of the taste layer epic (#1196): #1207, as decided: **suggest
only**. Bridge points out tool requests a person keeps answering the same way.
It grants or refuses nothing because of them. An allow points at chat access
settings. A decline can be written down as a code rule, which only asks agents
to do less.

## Functional Behavior

1. **Pattern.** A permission signal's request is its tool and a short pattern.
   - A shell tool (`Bash`, `shell`, `exec_command`, `run_shell_command`,
     `command`, `terminal`) gives its command's first two words, skipping
     leading `KEY=value` assignments and dropping a second word that is a
     flag.
   - Any other tool gives the top folder of the path it touches (`src/**`),
     the file itself at the root, or nothing.
2. **Qualify.** Per scope, tool and pattern:
   - at least 5 answers the same way (allowed, or declined);
   - from at least 2 chats;
   - no opposite answer in the last 30 days.
3. **Never suggest allowing** a network, destructive or credential command
   (by first word, risky pairs such as `git push` and `npm publish`, or
   secret-like words). Declines are always fine to suggest.
4. **Show** at most 3, most answered first. A dismissed suggestion (schema
   111, `taste_dismissed_suggestions`) never shows again.
5. **Methods:** `taste/list_permission_suggestions` and
   `taste/dismiss_permission_suggestion {key}`. The latter returns the list
   after the dismissal.
6. **Taste tab, "From your approvals":**
   - "You allowed Bash \`cargo test\` 6 times in 2 chats (octo/repo).", saying
     where to change chat access and that Bridge grants nothing on its own.
   - A decline offers **Make it a rule**, which writes the code rule "Don't run
     \`<pattern>\` without asking me first." in that repository (or every
     repository) and dismisses the suggestion.
   - **Dismiss** on every suggestion.

## Unit Tests

**Rust (`bridge-core/src/taste/permissions_tests.rs`, new)**
- `a_request_is_its_tool_and_a_short_pattern`
- `allows_are_never_suggested_for_network_destructive_or_credential_commands`
- `repeated_answers_across_chats_are_suggested_and_nothing_less`
- `a_recent_opposite_answer_or_a_risky_allow_is_not_suggested`
- `a_dismissed_suggestion_stays_dismissed_and_at_most_three_show`

**Vitest (`TasteRules.test.tsx`)**
- points out answers you keep repeating, grants nothing, and turns a decline
  into a rule

## Verification
`cargo test -p bridge-core --lib taste::`; the shell's contract test; Vitest
for `TasteRules`; lint and clippy ratchets.
