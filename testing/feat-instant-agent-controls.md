# feat/instant-agent-controls: Test Contract

## Functional Behavior

- Stop reacts on the next UI render. A send still being prepared is invalidated
  immediately and cannot start or submit a turn after Stop. Requests belong to a
  session and submission generation, including across chat switches.
- Stop settles durable turn state and closes the reader gate before process
  disposal. Late output cannot reopen cancelled work. Worker cancellation does
  not wait for the provider interrupt or hold the global adapter lock during
  disposal. Cancellation remains visible to the parent and releases leases.
- Steer remains available for ordinary chats on every provider. Native steering
  is used when supported; otherwise Bridge interrupts and runs the sanitized
  guidance next, ahead of ordinary follow-ups. Explicit Queue never interrupts.
- A stalled in-band interrupt cannot postpone the fallback deadline. Fallback
  observes the original turn/launch and queued message, and cannot kill a newer
  turn, revive a user-stopped session, duplicate guidance, or discard it silently.
- Codex uses `turn/steer` with `expectedTurnId`, preserving trusted application
  context and images, without sending a competing `turn/start`. Provider refusal
  is surfaced or recovered, never silently reported as accepted.
- Workers retain their typed result and checkpoint contracts. A non-native
  worker's guidance is durably queued and labelled accurately; stopping a worker
  remains immediate. Native Codex worker steering uses the same delivery seam.
- Stop, steer, preparation, and permission/question replies have reserved daemon
  capacity, including admission when background invokes saturate their budget.
- Worker guidance clears locally on submission, shows delivery pending, preserves
  text typed afterwards, and restores refused text without losing either draft.
- Main and aside composers use the same Steer/Queue policy and explain the
  interrupt-and-continue fallback without claiming the model has already read it.

## Research and Design Evidence

Primary sources checked on 2026-10-26:

- [Codex app server](https://learn.chatgpt.com/docs/app-server): steering appends to
  an active turn with an expected-turn precondition; interrupt completion is a
  separate event. Bridge must distinguish transport submission from completion.
- [Pi SDK](https://pi.dev/docs/latest/sdk): steering and follow-up queues have
  different consumption boundaries; abort waits for idle. Bridge keeps Queue
  explicit and separates immediate UI feedback from provider settlement.
- [Claude streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode):
  a persistent input stream supports queued messages and interruption.
- [OpenCode SDK](https://opencode.ai/docs/sdk/): abort is an explicit session
  operation. Bridge must bound its wait rather than depending on model output.

## Unit Tests

- `sessionStop`: pre-launch cancellation, independent session ownership, rejected
  cancellation, late acknowledgement, and fresh sends after a cancelled send.
- `sessionInput`: Steer stays Steer without native capability; Queue is unchanged.
- `SteerComposer`: optimistic clear, pending indication, refusal restoration,
  preserving a newer draft, IME and session-switch behavior.
- `codex_adapter`: native steer params, expected turn, context and images, idle
  refusal, and wire method selection.
- `daemon_host`: control lanes are disjoint from background/GitHub lanes;
  saturation of ordinary invoke admission leaves control capacity available.

## Integration / Functional Tests

- `live_turn`: explicit Steer on a non-native chat interrupts and queues at the
  front; Queue does not interrupt; native delivery uses the steering seam;
  fallback cannot affect another turn or revive a cancelled session.
- `live_turn`: worker Stop retires its reader and runtime without invoking a
  blocking interrupt, preserves cancelled result/reporting and other sessions.
- `App.instantSend`: Stop during held preparation prevents startup/submission;
  pending feedback and ordinary send/retry behavior remain correct.
- Relevant existing worker, queue, images, transcript, and archive tests pass.
- `bun run build` and `bun run test` must both pass before PR creation.

## Smoke Tests

- Build the app and daemon, exercise Steer and Stop with a stalled fake runtime,
  and confirm control requests do not depend on model tokens or background reads.
- Review each committed step and the cumulative diff against this contract.

## E2E Tests

- Use the browser demo with the real React UI to verify immediate send/stop
  affordances and composer behavior. Provider correctness is covered by local
  process/transport fixtures; no paid model call is required.

## Manual / cURL Tests

- In a working direct chat, submit guidance with Steer selected. The bubble must
  appear immediately. Native providers receive guidance in place; others redirect
  through cancellation and continuation. Queue waits without cancelling.
- Stop during startup and while a tool is busy. No pending startup may submit
  afterwards, and no late frame may return the stopped turn to working.
- Stop a worker while another chat runs. Only the target stops and its parent
  sees cancellation. Repeat send, steer, Stop and switch chats.
- These UI journeys supplement, and do not replace, deterministic race tests.
