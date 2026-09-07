# feat/336-model-switch-handoff — Test Contract

Issue: https://github.com/Atharva-Kanherkar/bridge-harness/issues/336
"In-chat model/harness switch drops the entire conversation — the new model should get an LLM-summarised, typed handoff brief."

## Functional Behavior

Concrete behavior this branch must deliver (re-derived against current `main`, since the issue's line numbers predate 48 commits of drift):

1. **Switching model within the same harness** from the chat header: the provider session is kept and the next turn resumes it natively under the new model — see `testing/fix-same-harness-model-switch-native-resume.md` (#527), which supersedes the earlier restoration-context behavior described here. A question that depends only on earlier turns ("what file did we decide to change?") is answerable without re-explaining.
2. **Switching across harnesses** (codex → claude → opencode): same carried context, and the injected instructions are explicitly labelled projected stored history — "Bridge checkpoint-restoration context (stored history, not native provider resume)" — never a claim of native resume.
3. **Summarise before teardown, best-effort and bounded**: when the outgoing provider process is hot and idle, the switch first asks it for a typed compaction-style summary using the existing validated pipeline (`compaction.requested` → internal turn → parse/validate → one repair retry → `checkpoint`+`compaction` entries). The wait is deadline-bounded; on timeout/failure the pending request is cancelled via `compaction.failed` and the switch continues.
4. **Fallback is the mechanical projection**: if the outgoing provider is not running, already gone, produces invalid output twice, or exceeds the deadline, the switch completes anyway and the incoming model gets `restoration::checkpoint_context` (recent user/assistant messages, worker results, any prior summary). Since #529 this is sized by a byte budget derived from the incoming model's context window (`restoration::restoration_budget_bytes`), not the former fixed 8 KB trim, and the outgoing model's summary now lands in the background rather than on the switch's critical path — see `testing/feat-handoff-budget-and-async-switch-summary.md`.
5. **No unvalidated model prose reaches the new provider's instructions**: only output that passed `Checkpoint::parse_and_validate` (fences/trailing prose/wrong sourceAgent rejected) lands in the forest as a summary; the injected context text is Bridge-generated from validated entries.
6. **`$codex <question>` from inside a chat** creates the sibling chat AND seeds it with a durable `handoff.brief` entry projecting the source session's stored context, so the new harness's first turn knows what "this" refers to. Sessions without carriable history create exactly as before.
7. **The `session.model_changed` transcript event states what was carried forward** — e.g. "carried forward: summary + N decisions + M files" vs "no context carried (summary unavailable)" — in both its detail text and structured `data.carriedContext`.
8. **Fidelity is recorded for switched chats**: after a *cross-harness* switch, the session's `continuation_fidelity` reflects projection (ProjectedAtBoundary / ProjectedMidTurn), so the existing UI fidelity banner tells the truth. Brand-new chats keep `native` — and since #527 so do same-harness switches, whose next turn resumes the provider session instead of projecting.

Explicitly out of scope (unchanged): same-harness native restarts (`RestorationPlan::Native` path — now delivered by #527, see `testing/fix-same-harness-model-switch-native-resume.md`), worker handoff, compaction triggers/policy.

## Unit Tests (Rust, `cargo test -p bridge-core`)

Restoration / forest layer:
- `handoff_brief_entries_feed_the_checkpoint_projection` (`restoration.rs`) — appending `handoff.brief {text}` makes `checkpoint_context` return the text inside the labelled envelope.
- `handoff_brief_payload_requires_text` (`session_forest.rs`) — payload validation rejects an empty/missing `text`.

Prompt compilation:
- `session_prompt_injects_restoration_context_like_the_orchestrator` (`live_turn.rs` prompt_section_tests) — `compile_session_prompt(Some(context))` emits a `restoration_context` section whose bytes match what `compile_orchestrator_prompt` produces for the same input.

Handoff carry:
- `carry_handoff_projects_the_source_branch_into_the_target` (`handoff.rs`) — source session with history → target gains one `handoff.brief` entry containing the projected text and source ids; empty source → no entry.
- `carry_handoff_refuses_self_and_unknown_sessions` — carrying into/out of the same session or unknown sessions is rejected/no-op without corrupting either forest.

Model-switch summary pipeline:
- `model_switch_summary_is_skipped_without_a_hot_provider` (`sessions.rs`) — no live runtime → returns mechanical outcome, appends nothing, leaves no pending compaction behind.
- `model_switch_summary_timeout_cancels_the_pending_request` — with an already-pending request and an elapsed deadline, the settle step records `compaction.failed` and `pending()` clears (the switch must not strand a pending request that would later misparse normal replies).

Commit reporting:
- `commit_reports_carried_context_from_the_projection` (`sessions.rs`) — chat with seeded history commits with `data.carriedContext.summary = true` and detail naming the counts; empty chat reports `carriedContext: null` / "no context carried".

## Integration / Functional Tests

- Full adapter round-trip (real LLM summary during switch): N/A in cargo unit scope — no test fixture can spawn provider processes (every stub's `start()` errors by design). Covered instead by the piecewise tests above plus the timeout/cancel test; the happy path reuses the production compaction pipeline that has its own coverage (`handle_output` repair/complete tests).
- Frontend (Vitest): model-switch fidelity label rendering covered in `AgentConversation.test.tsx` (projected banners) extended with the `session.model_changed` carried-context wording.

## Smoke Tests

- `bun run build` green (tsc -b && vite build).
- `cargo check` green.
- Existing suites unaffected: `bun run test`, `cargo test` green.

## E2E Tests

N/A — no automated desktop E2E harness exists in this repo. Manual verification below stands in.

## Manual / cURL Tests

Desktop app (`bun run tauri dev`) manual script:
1. Start a direct chat, exchange a few messages establishing a fact ("we decided to change src/App.tsx"). Use the header control to switch codex → claude. Ask "what file did we decide to change?" — claude answers from carried context without re-explaining.
2. Switch again claude → opencode mid-conversation; ask a follow-up referencing earlier turns — context holds; transcript shows the `session.model_changed` entry stating what was carried.
3. Stop the session (provider cold), switch harness, send a message — switch completes instantly (no summary wait), message starts fine.
4. Type `$codex what were we doing?` inside an active non-codex chat — new codex chat opens and answers about the prior conversation.
5. Switch while the transcript shows the fidelity banner — banner reads "phase-boundary projection" wording after the switch's next start.
