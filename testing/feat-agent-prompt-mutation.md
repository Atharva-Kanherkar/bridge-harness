# Agent prompt mutation — implementation contract

Research completed against main at `1cffffc8`. This contract is written before
implementation. The baseline is the adaptive-learning audit; this feature adds
an approved prompt mutation path, not a learning loop.

## Product behavior

- Add an initially empty `additional_guidance` section to the Bridge
  orchestrator and five worker role prompt stacks. Preserve dynamic built-in
  contracts; empty guidance contributes no extra compiled section or bytes.
  The new host-tool instructions themselves change the orchestrator prompt.
- A Bridge orchestrator may propose appending guidance to its own role or the
  role of one of its own worker sessions. A worker may propose for its own role
  only when that role is explicitly enabled in Settings. Direct chats, hidden
  evaluator/maintenance sessions, unrelated workers, and spoofed identities
  have no mutation authority.
- These targets are shared role defaults. They are not per-agent-definition
  prompts or provider-owned base prompts. The approval must state this scope.
- The provider-neutral host tool uses a strict `bridge-prompt-change` assistant
  control fence, consistent with Bridge's existing delegation/control protocol.
  It carries schemaVersion=1, requestId, optional targetSessionId (omitted means
  self), guidance, and rationale. It cannot carry actor identity or permissions.
- Every proposal requires human approval of exact escaped before/after bytes.
  Provider permission auto-approval and accept-for-session cannot authorize it.
- Approval persists the append and attribution atomically. Decline changes no
  prompt. A stale proposal is resolved as stale without overwriting newer work.
- Changes apply the next time Bridge starts or relaunches a matching role.
  Running turns retain their launch instructions; approval does not force an
  interrupt or claim an immediate model change.
- Prompt Studio remains the editing/history/restore surface. Restore is a new
  immutable revision and cannot silently erase an intervening audit record.

## Persistence and authority

- Host derives actor session, turn, role, target role, and worker ownership from
  persisted runtime state. Worker proposal role grants are stored in
  PermissionPolicy.workerPromptProposalRoles, default empty.
- A distinct deterministic prompt-mutation decision requires approval or
  rejects; ranking, prompt text, model-chosen paths, and provider permission
  policy cannot grant mutation authority. Recheck actor/target relationship and
  current role grant before acceptance. Completed sessions do not acquire new
  authority; an already-created proposal can be reviewed without a live model.
- Bound request identifiers, rationale, additions, total guidance and pending
  proposals. Reject unknown fields, malformed/multiple/nested control fences,
  and content that fails existing prompt-compiler validation.
- Snapshot current section state, latest revision id and exact text hash.
  Compare all at approval inside the same write transaction as active-state
  update, attributed revision, proposal settlement and approval resolution.
- Dedupe by actor session/turn/request id. Identical replay returns the existing
  result; conflicting reuse is rejected. One proposal per actor turn is enough.
- Revisions remain append-only, including actorSessionId, actorTurnId,
  actorRole, proposalId and rationale for an agent-originated change. Preserve
  old history and expose attribution as an optional field in the typed wire view.

## Runtime and UI

- Intercept only an authenticated live assistant completion; exclude tool
  output, user text, quoted examples and maintenance turns. Strip the control
  payload from prose after processing; show a real approval card.
- A worker waiting on this host tool must not be classified as a malformed
  worker result. Surface its approval in the parent's conversation/inbox.
- Return a host-authored outcome after resolution without bypassing the normal
  session input/lifecycle gates. If its runtime has ended, preserve the durable
  outcome and do not resurrect a finished worker merely to acknowledge it.
- Restart/stop behavior must not leave invisible, permanently pending cards.
- Existing approval event kind is reused with approvalType=prompt_mutation.
  Review fields: proposalId, target, sectionId, operation=append, beforeText,
  afterText, appendedText, rationale, actorSessionId, actorTurnId, actorRole,
  baseRevisionId, baseHash, effect=next_launch.
- Actions are Approve change and Decline. Show stale and already-resolved
  outcomes correctly; render prompt content as text, never executable markup.

## Verification

- Storage: empty guidance preserves legacy compiled bytes; dynamic worker
  depth survives appends; exact append, size/compiler validation, rollback,
  concurrent edits, reset/restore ABA, idempotency/conflicting replay,
  attribution immutability, migration upgrade and transaction-failure rollback.
- Policy/runtime: actor spoofing, own/unrelated child targets, default worker
  denial and explicit opt-in, revoked grant, hidden/direct session denial,
  duplicate events, auto-approve bypass prevention, host-control-turn handling,
  approved/declined/stale delivery, and restart/terminal behavior.
- UI: exact escaped before/after, clear shared-role scope/timing, actor/rationale,
  no accept-for-session, correct action arguments, busy state and conflict,
  attributed revision history and restore, role opt-in settings.
- Regenerate protocol artifacts; run focused Rust/frontend tests, full
  `bun run build`, `bun run check`, and `bun run test`. Perform browser visual
  verification of the approval/history/settings flow using fixture data.

## Browser fixture

Run `bun run dev`, then open
`/testing/fixtures/prompt-mutation-preview.html` on that server. This renders
the real conversation approval component with a synthetic host event. Check
the literal before/after text, rationale, shared-role scope and next-launch
timing, then approve or decline. `window.lastPromptReviewAction` exposes the
submitted event id and decision. Add `?outcome=stale` to exercise a concurrent
edit response. The fixture does not start a provider or write durable app data.

In the browser app, also check Settings → Permissions with all worker proposal
roles initially disabled. Enable one role, then open Settings → Prompts and
select Additional guidance. Save, reset and restore a revision; the editor
and history must agree after each action.

Node 25 enables a native Web Storage global that conflicts with the existing
Vitest navigation fixture. In that environment, run the test script with
`NODE_OPTIONS=--no-experimental-webstorage bun run test` so the suite uses its
intended jsdom storage implementation.

## Validation recorded on 2026-09-09

- `bun run build` and `bun run check` passed.
- The full test script passed with the Node option above: 50 sidecar tests,
  1,876 frontend tests and 2,409 native tests including doctests. The existing
  skips remain: one sidecar test and 13 native tests.
- A focused run passed 52 prompt-mutation, migration and permission-gate
  regressions before the final full run. This includes repeated migrations with
  attributed history, failed-worker settlement, parent queue boundaries,
  recovered outcomes and duplicate prevention.
- Browser checks exercised approve, decline and stale outcomes with literal
  markup-like text, default-disabled worker grants, and guidance save/reset/
  restore. No browser errors were reported. These checks used fixture/mock
  data; no live provider smoke test was performed.
- Final diff whitespace checks passed. The implementation was reviewed by
  separate storage/policy, runtime and UI agents; their identified defects were
  fixed before the passing final run.

## Research basis

- `prompt_sections.rs`: existing atomic override/revision and restore store.
- `prompts.rs` and `delegation.rs`: worker defaults depend on topology depth.
- `live_turn.rs`: shared assistant control protocol and prompt hash checks at
  launch; ordinary hot message submission does not recompile instructions.
- `agent_config.rs`: custom agent prompts are currently unversioned and worker
  prompt selection is role-based, with no durable configured-agent binding.
- [Claude system prompts](https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts):
  provider presets and application additions are distinct layers.
- [Claude permissions](https://code.claude.com/docs/en/agent-sdk/permissions):
  SDK permissions govern provider tools; Bridge config approvals need their own gate.
- [SQLite transactions](https://www.sqlite.org/lang_transaction.html):
  compare and commit within a single write transaction.
