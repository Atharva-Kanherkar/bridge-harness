# taste-signals — Test Contract

Sub-issue 2 of the taste layer epic (#1196): #1198. Suppression, the rule
learner, rule scores and the replay eval all need the same facts: what a
person did with what agents produced. This adds one append-only log of them
and its producers. Nothing reads it yet.

## Functional Behavior

1. **Schema 106.** `taste_signals(id, at, scope_key, source, verdict,
   subject_id, harness, path, language, severity, category, title, body,
   edited_body, fingerprint, job_id, pr_number, session_id, detail_json,
   superseded_by)`. `source` is `review_decision | review_fixed |
   review_thread | permission`; `verdict` is `kept | edited | dropped | fixed
   | resolved | ignored | allowed | declined`. At most one live (not
   superseded) signal per `(source, subject_id)`.
2. **Review decisions** are written by `reviews::store::decide` in the same
   transaction as the decision: a failed decide writes none. The same decision
   again adds nothing; a changed one (including a different edited body)
   appends a signal and marks the earlier one superseded by it.
3. **Fixes.** When a re-review marks an earlier kept or posted finding fixed
   (`mark_fixed`), a `fixed` signal records the fixing head.
4. **Threads.** Preparing a review of a PR reads its comment threads; an
   earlier posted finding whose thread is resolved gets `resolved`, and one
   whose thread went outdated with no reply, no fix and no drop gets
   `ignored`. Best effort: a failure is logged and the review goes on.
5. **Permissions.** A person's answer to a tool request in a chat
   (`resolve_provider_permission`, actor `human`) is `allowed` (accept, accept
   for session) or `declined`; a cancel writes nothing. The title is the tool,
   the body its command or input (600 characters at most), and the path its
   file when it names one.
6. **Scopes.** Review signals belong to `project:<owner>/<repo>` of their job.
   A chat's signals belong to its workspace's reviewed repository, else
   `workspace:<id>`, else `account:local`.
7. **Text is safe.** Titles, bodies and edits are secret-redacted, trimmed and
   capped at 4000 characters. Every `at` is stored in one format (UTC,
   milliseconds, `Z`), so string order is time order whoever wrote it, and a
   fix is dated by the re-review that found it.
8. **Category and language.** A keyword classifier reads the title first, then
   the body, into `security | correctness | error-handling | tests |
   performance | naming | docs | style | other`; the language comes from the
   path's extension.
9. **Backfill.** The migration copies every decided finding and every fix
   already stored, dated when decided. Running it again adds nothing.
10. **Retention.** The memory maintenance tick deletes signals older than 365
    days.

## Unit Tests

**Rust (`bridge-core/src/taste/signals_tests.rs`, new)**
- `the_log_exists_with_one_live_signal_per_subject`
- `deciding_findings_writes_one_signal_each_in_the_repository_scope`
- `a_changed_decision_supersedes_and_the_same_one_adds_nothing`
- `a_decide_that_fails_writes_no_signal`
- `a_secret_in_an_edit_is_stored_redacted`
- `backfill_copies_what_was_decided_before_the_log_once`
- `a_re_review_that_fixes_a_kept_finding_records_the_fix`
- `thread_outcomes_are_resolved_or_ignored_and_nothing_else`
- `permission_decisions_are_allowed_or_declined_in_the_chats_scope`
- `a_person_declining_a_tool_request_in_a_chat_is_logged` (through
  `api::resolve_approval`)
- `signals_older_than_a_year_are_swept`
- `categories_read_the_title_first`
- `languages_come_from_the_extension`
- `every_instant_in_the_log_has_one_format`

## Verification
`cargo test -p bridge-core --lib` for `taste::`, `reviews::`, `store::`,
`permission`, `approval` and `memory_consolidation` green.
