# taste-chats — Test Contract

Sub-issue 8 of the taste layer epic (#1196): #1204. The `code_prefer` rules a
person approved go into every new chat's memory packet for that repository,
so Claude, Codex and OpenCode write code the way their reviews ask for.

## Functional Behavior

1. **Repository of a chat.** A chat's workspace names its repository from its
   latest review job, else from its checkout's `origin` remote. The remote is
   read from `.git/config`, following `gitdir:` and `commondir` for a linked
   worktree, without running git, and only GitHub remotes count. A chat with
   no workspace, or an unnamed one, has no repository. The decision log's chat
   signals use the same resolution.
2. **Rules.** A chat gets active `code_prefer` rules of its repository and of
   every repository: the repository's first, then the most trusted, then the
   most recent, at most 8. Paths are shown on the line, not filtered.
   `review_avoid` and `review_prefer` rules, waiting rules and other
   repositories' rules never go to a chat.
3. **Packet.** After the memories, under one heading, each rule is a line
   `[<id8>] code rule (paths: …): <rule>`. It counts against the packet's
   budget, and a rule that does not fit is excluded as `over_budget`. A packet
   with only code rules has no memory heading. A chat with no code rules gets
   a byte-identical packet. Turning memory injection off removes the rules
   too.
4. **Audit.** The packet audit records each rule sent, with reason `your code
   taste`.

## Unit Tests

**Rust (`bridge-core/src/taste/chat_rules_tests.rs`, new)**
- `a_chat_gets_its_repositorys_code_rules_and_every_repositorys_and_nothing_else`
- `a_chat_with_no_code_rules_gets_the_packet_it_always_did`
- `code_rules_alone_still_make_a_packet_and_the_memory_switch_turns_them_off`
- `at_most_eight_code_rules_and_never_past_the_packet_budget`
- `a_chat_in_an_unreviewed_checkout_is_placed_by_its_origin_remote`

**Rust (`bridge-core/src/taste/repository.rs`)**
- `the_origin_remote_names_the_repository`
- `a_linked_worktree_reads_its_main_repositorys_config`

## Verification
`cargo test -p bridge-core --lib` for `memory` and `taste::`.
