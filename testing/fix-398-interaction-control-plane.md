# fix-398-interaction-control-plane — Test Contract

## Functional Behavior

### Typed interactions and honest policy

- Normalize provider interactions as distinct `permission` and `question` contracts with stable provider request identity, provider-supported actions, and typed resolution state.
- Render permission actions only when the provider offered the corresponding option. Never synthesize reject, allow-once, or allow-for-session actions.
- Route permission decisions through provider permission reply channels and question answers/rejections through their dedicated question channels and wire formats.
- Rename the bypass setting to **Auto-approve provider permissions** and describe its actual scope consistently for Claude, Codex, OpenCode, and ACP/Cursor. Questions, operating-system prompts, and non-provider authorization gates are outside its scope.
- Automatic permission policy must prefer an exact offered `allow_always` option ID, then an exact offered `allow_once` option ID. If neither exists, leave the permission pending and explain why; never invent or approximate an option.
- A policy-resolved permission must never expose actionable buttons. Inline resolution identifies the actor (`human` or `policy`), friendly outcome, and policy reason.
- Saving the permission policy gives explicit success or failure feedback.

### Single-owner interaction resolution

- Claim a pending interaction atomically before sending a provider response. Exactly one resolver may own a request across double clicks, concurrent windows, policy-versus-human races, and restarts.
- A winning resolver produces at most one provider response and one durable terminal resolution.
- Repeated or losing attempts return a typed `alreadyResolved` result containing the durable resolution instead of sending another provider response or raising an untyped error.
- Provider failure, conflict, cancellation, success, and automatic resolution remain durable, non-actionable where terminal, and visible inline.
- The UI enters a local settling state immediately, disables every action while the returned promise is pending, prevents duplicate dispatch, and shows progress, inline failure, or conflict recovery.

### First-class source-scoped asides

- `sessions/create_aside_chat` accepts an exact `sourceSessionId` and atomically returns `{ state, sessionId, handoffStatus, fidelity }` for the created aside.
- The aside inherits its source session workspace, cwd, and explicit permissions. Creation never falls back to an unrelated scratch workspace.
- Aside lifecycle state is scoped to the source chat and records deterministic phases for creating, handing off, ready, switching (queued, summarizing, stopping), failed, cancelled, and closed.
- Concurrent session creation cannot redirect handoff or the first message because callers use the exact returned session ID.
- Failed create, start, or send preserves/restores the full draft and all attachments. Duplicate submit is blocked visibly while delivery is pending.
- Handoff status and fidelity are shown before the first answer. Failures provide retry/recovery instead of being swallowed.
- Navigating to another source chat cannot resurrect an unrelated aside. Close/reopen/promote continues to use the exact source/aside relationship.
- Model switching shows its current stage, timing expectation, failure, cancellation, and retry path; it does not hide a long summary/teardown behind a generic disabled picker.

### Descriptor-driven Cursor integration

- Every applicable frontend model picker derives runtimes/models from backend adapter descriptors and includes Cursor when advertised; no provider-name allowlist controls visibility.
- Model commands send exact ACP-advertised Cursor model IDs.
- Cursor catalog order is not interpreted as capability rank. Models use deliberate metadata when present or remain unranked.
- One backend-owned compatibility predicate governs settings choices, routing, and adapter launch. Unsupported Cursor role/write-mode combinations cannot be saved, routed, or launched inconsistently.
- Cursor remains compatible with supported direct/aside work and worker configurations.
- One deadline covers the complete ACP `initialize` plus `session/new` handshake. Timeout/error paths stop and reap the spawned child process.

## Unit Tests

- Provider/manual/policy matrix for Claude, Codex, OpenCode, and ACP/Cursor proves typed kind, supported actions, policy behavior, actor/reason attribution, and no unsupported outgoing action.
- ACP permission policy test for `session/request_permission` selects the exact offered positive ID, preferring `allow_always` over `allow_once`, publishes no actionable policy-resolved state, and resumes the session.
- OpenCode question, Codex user-input, and MCP elicitation tests prove question-specific payloads and prove the permission resolver cannot answer questions.
- Barrier-based resolver test starts two concurrent attempts on one request and asserts one claim, one provider reply, one durable resolution, and one typed `alreadyResolved` loser.
- Restart/replay tests prove a terminal durable resolution cannot be delivered again.
- Frontend deferred-promise tests prove the first click disables all actions, progress is inline, the second click does not dispatch, and rejection restores a recoverable card with inline error.
- Frontend policy rendering tests prove a policy-marked item never renders action buttons and identifies automatic actor/reason in friendly language.
- Frontend provider-action tests prove absent options never render and exact option IDs flow through callbacks.
- Policy settings tests prove honest copy and explicit save success/failure feedback.
- Aside reducer/component tests cover source scoping, exact returned ID, visible create/handoff/switch stages, navigation isolation, cancellation, retry, and draft plus attachment restoration.
- Cursor descriptor/model-picker tests prove advertised runtimes and exact model IDs appear without a hardcoded allowlist or positional tier guessing.
- Cursor compatibility tests use the same predicate across settings payload validation, routing, and launch.
- ACP handshake test makes `initialize` succeed and `session/new` stall, then asserts bounded failure and child reaping.

## Integration / Functional Tests

- Permission flows cover Claude, Codex, OpenCode, and ACP/Cursor under manual and auto-approve policy with differing provider-offered action sets.
- Interaction event serialization/deserialization preserves kind, exact supported actions, provider identity, actor, reason, settling/terminal state, and typed already-resolved results through Rust protocol and TypeScript consumption.
- Aside daemon lifecycle covers create → handoff → start → reply → switch → follow-up → close → reopen/promote while retaining source workspace access.
- Concurrent aside creation proves handoff and first delivery target the exact returned aside session.
- Provider startup and send failures prove the draft text and every attachment remain available for retry.
- Descriptor health output, frontend selection, routing, and adapter launch agree on Cursor models and compatible roles.

## Smoke Tests

- `bun run build` passes.
- `bun run test` passes, including Vitest and the complete Rust workspace.
- A Cursor direct chat is selectable from advertised descriptors and can send an exact advertised model ID.
- Enabling Auto-approve provider permissions resolves a real/simulated Cursor `session/request_permission` without exposing actionable UI.
- A repository chat can create an aside that reports its exact session ID and uses the same repository cwd/workspace.

## E2E Tests

- Manual provider journey: provider requests permission → only offered actions appear → one action settles inline → friendly durable human resolution appears.
- Automatic provider journey: policy is enabled → provider permission resolves before actionable publication → inline policy actor/reason appears → turn continues.
- Question journey: provider asks a question → question-specific answer/reject controls appear → correct wire reply is delivered → no permission decision is sent.
- Aside journey: source repository chat opens an aside → handoff status/fidelity appears → message succeeds in the source cwd → model switch stages appear → failed follow-up preserves the draft → retry succeeds → navigation cannot show it on another source.
- Cursor startup journey: discovery either completes `initialize` and `session/new` within the single deadline or reports bounded failure after reaping the child.

## Manual / cURL Tests

- Inspect adapter health/descriptors in the running app and confirm Cursor plus its exact ACP model IDs appear in each applicable picker.
- Exercise settings for every worker role and confirm incompatible Cursor role/write-mode combinations are unavailable and rejected consistently by backend validation.
- Toggle Auto-approve provider permissions and confirm success feedback plus copy that excludes questions and OS prompts.
- Trigger one provider permission and one provider question; verify their distinct cards, controls, wire channels, and inline terminal states.
- Double-click a permission in two windows and verify one provider response; the later attempt shows the durable already-resolved outcome.
- From a repository session, create an aside and verify its persisted `workspace_id` and `cwd` equal the source session values.
- Stall scripted ACP `session/new` after successful `initialize`; verify the configured complete-handshake deadline terminates discovery and no child remains.

