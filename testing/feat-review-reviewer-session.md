# feat/review-reviewer-session: Test Contract

Sub-issue 3 (#1118) of the standalone PR reviewer epic (#1115). The epic and #1118 are the
authority; this file names what this branch must prove. It builds on the runner (#1124) and the
prepared checkout and context (#1126).

Note on order: this contract was drafted during a first exploration spike (adapter survey, the
findings parser and the session loop), and is committed before any of that code.

## Scope decisions

- **Per-harness read-only start, without new chat behaviour.** A new `start_reviewer` on the
  adapter trait and registry. Its default is a read-only start inside an offline
  `ReadOnlySandbox` (Claude: Read, Grep, Glob and the sidecar's read-only set, plus writes only
  in the sandbox's output folder; Codex: read-only mode with a no-network sandbox policy).
  OpenCode overrides it with a `Reviewer` session kind: deny everything, allow `read`, `grep`,
  `glob` and `list`. Chat permission rules do not change.
- **`ReadOnlySandbox::offline(workspace)`**: the existing sandbox with tool network off, without
  a `DelegationRequest`.
- **The context is copied into the checkout** as `.bridge-review-context/`, so every harness
  reads it with relative paths and OpenCode never needs `external_directory`.
- **Every permission request is declined at once** with `decline` (the one decision all three
  adapters accept), and questions are rejected. Each is counted on the pass as
  `denied_requests`.
- **The session loop sits behind a `Transport` trait**, so denials, limits, cancellation and the
  repair turn are tested with a scripted session. The registry is the production transport.
- **Default review instructions** live in `reviews::session::REVIEW_INSTRUCTIONS` (no `gh`,
  nothing about posting). A custom reviewer prompt is used as written, with `{number}` expanded
  and a line telling the reviewer to skip any posting instructions. The legacy
  `reviewer_settings::DEFAULT_SYSTEM_PROMPT` still serves `github/github_review` until the pane
  switches over (#1120).
- **Errors map to the runner:** rate limits, overload and timeouts are transient. A start
  failure, the process ending, a provider error or the wall clock all move the pass to a fallback
  harness. The tool cap or a contract failure after the repair turn is fatal.
- **No session rows and no resume.** Harness transcripts stay wherever the harness keeps them;
  Bridge never resumes them.
- Posting stays unavailable until #1119.

## Amendments after review 6023548164

- **The context folder is never planted.** It is `.bridge-review-<12 random hex>`, made with a
  create that fails if the path exists, and each file is created new (`create_new`), so a symlink
  the PR committed is refused rather than followed. Its name appears only in the per-PR turn.
- **An OpenCode reviewer loads nothing executable from the checkout.** Its server runs with
  `OPENCODE_DISABLE_PROJECT_CONFIG`, `OPENCODE_CONFIG_PROJECT_DISABLE`,
  `OPENCODE_DISABLE_CLAUDE_CODE`, `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` and
  `OPENCODE_DISABLE_AUTOUPDATE`, and with `lsp` and `formatter` off, merged into the person's
  own inline config. Claude's read-only mode already loads only the sanitized user settings (no
  project hooks, MCP or plugins), and Codex runs with an isolated `CODEX_HOME`.
- **Tool calls are distinct invocations** of `tool.started` and `command.started` (Codex), by
  item id, shared across both turns.
- **Questions are refused on their own channel:** Codex `item/tool/requestUserInput` with empty
  answers, `mcpServer/elicitation/request` with `{"action":"decline","content":null}`, and
  otherwise `reject_question`.
- **Usage survives failure and retries.** Each attempt adds its usage to the pass's
  `UsageMeter`; the runner writes the total whether the pass completes or fails.
- **Normalizer state is released** (`forget_session`) when a reviewer session stops.
- **The diff parser follows hunk line counts**, so `---` and `+++` content lines stay content, and
  it decodes Git's octal-quoted names.
- **Test git calls go through `git::git_command`**, so the spawn scanner passes.

New regressions:
- `reviews::session::tests::a_planted_path_in_the_checkout_is_never_followed`
- `reviews::session::tests::each_invocation_counts_once_and_codex_commands_count_too`
- `reviews::session::tests::a_failed_pass_still_records_what_it_spent`
- `reviews::contract::tests::header_like_lines_inside_a_hunk_are_content`
- `reviews::contract::tests::octal_quoted_names_decode_to_the_real_path`
- `reviews::contract::tests::real_git_output_round_trips_through_the_parser`
- `opencode_adapter::tests::a_reviewer_server_loads_nothing_executable_from_the_checkout`
- `opencode_adapter::tests::live_reviewer_never_runs_a_planted_plugin` (ignored, run by hand
  under `env -i`: a planted plugin calls a loopback canary from a chat session and never from a
  reviewer session)

## Functional Behavior

### Session (`reviews/session.rs`)

- Limits: 900 s wall clock and 300 tool calls by default, shared by the review turn and the
  repair turn. Cancellation is checked at least every 200 ms and stops the harness process.
- Usage: input and output tokens from `usage.updated` events, cost from `totalCostUsd` (Claude)
  or `cost` (OpenCode); Codex reports none.
- The final message is the last non-empty assistant message, or the turn result when there is
  none.
- Contract failure gets one repair turn on the same session, quoting the error. A second failure
  fails the pass with that error.
- Cache-friendly order. The instructions are the stable prefix: the hiding rule when on, the
  offline role, the repository's convention files (`AGENTS.md`, `CLAUDE.md`,
  `.github/copilot-instructions.md`, `CONTRIBUTING.md`, only those present), the review rules,
  and the output contract. The turn carries the PR number and title, the context paths, the file
  count and the files the diff left out. Nothing per-PR is in the prefix.
- Model and effort: the reviewer settings for the harness, then the Reviewer model profile when
  it names this harness, then the harness default at `high` effort.

### Contract (`reviews/contract.rs`)

- Exactly one fenced `json` block in the final message, prose around it allowed. Unknown fields,
  wrong types and bad enums are errors.
- Validation never fails a pass. A finding is inline when its path is a changed file and its
  `line` (and `startLine`, when it is below `line`) fall in one hunk on its `side`. RIGHT is new
  and context lines; LEFT is old and context lines. Anything else is kept as `outside_diff`.
- Confidence is clamped to 0 to 1, titles are cut to 120 characters, empty titles are dropped,
  and `(path, line, normalized title)` repeats collapse. At most 50 findings are kept, ranked by
  severity then confidence, with inline findings first.
- `fingerprint = sha256(path, normalized title, hunk text)`. An empty hunk is used outside the
  diff.
- The pass summary names the files the diff left out.

## Unit Tests

`reviews::contract::tests`:
- `the_json_block_is_found_inside_prose`
- `a_missing_or_doubled_block_is_an_error_a_repair_turn_can_quote`
- `fields_are_strict`
- `findings_inside_a_hunk_are_inline_and_the_rest_move_to_the_summary` (multi-hunk, deleted
  lines on LEFT, ranges, new and deleted files, binary, unchanged file)
- `hunks_track_both_sides_line_numbers` (including quoted paths)
- `duplicates_collapse_and_ranking_is_severity_then_confidence`
- `titles_are_cut_and_fingerprints_follow_the_hunk_not_the_wording`
- `oversized_and_empty_reviews_validate`

`reviews::session::tests` (scripted transport):
- `a_clean_turn_returns_the_review_and_its_usage`
- `permission_requests_and_questions_are_declined_at_once_and_counted`
- `a_broken_answer_gets_one_repair_turn`
- `an_answer_still_broken_after_the_repair_fails_the_pass`
- `the_tool_cap_and_the_wall_clock_end_the_session`
- `a_cancel_stops_a_hung_session_quickly`
- `errors_tell_the_runner_whether_to_retry_or_fall_back`
- `the_stable_prefix_holds_nothing_that_varies_per_pr`
- `a_pass_reads_the_context_and_keeps_inline_and_outside_findings_apart`

`opencode_adapter::tests`:
- `the_reviewer_reads_and_nothing_asks_while_chat_rules_stay_as_they_were` (chat read-only
  still asks before bash; reviewer rules allow only read, grep, glob and list, and never ask;
  v2 conversion)

`worker_sandbox::tests`:
- `an_offline_sandbox_denies_tool_network`

## Integration / Functional Tests

- `bridged/tests/reviews.rs`, fully offline: a retried job prepares a real checkout and runs
  Bridge's real Claude sidecar against the fake SDK (`testing/fixtures/workflow-e2e`). The fake
  SDK's reviewer branch reads the context, tries a `Bash` call that the read-only boundary
  refuses, and answers with one finding on the PR's changed line. The job ends `drafted` with
  that finding inline, and no review checkout is left.

## Live Tests (run by hand, `--ignored`)

- `reviews::session::tests::live_review_on_a_fixture_pr`, with
  `BRIDGE_REVIEW_LIVE_HARNESS=claude|codex|opencode` (and `BRIDGE_REVIEW_LIVE_MODEL` for
  OpenCode). It runs the real harness on a fixture repository with one planted bug and asserts a
  valid findings set, no shell tool (Claude, OpenCode), and no pending permission. It prints the
  tools used and the usage.

## Smoke Tests

- `cargo test -p bridge-core --lib reviews`, `opencode_adapter`, `worker_sandbox` green.
- `cargo test -p bridged --test reviews` green.
- `bun run build` green.

## E2E Tests

N/A beyond the daemon test above: the pane does not start review jobs until #1120.

## Manual

- The OpenCode stall on `main`: `permission_rules(Some(WriteMode::ReadOnly))` asks before
  `bash`, and the old reviewer prompt requires `gh pr view`, `gh pr diff` and `gh pr comment`.
  Recorded in the PR with the exact rules and prompt.
