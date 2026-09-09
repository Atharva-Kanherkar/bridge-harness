# Streaming, memory retention, and chat cancellation

Started from fresh main `006507c` and merged main `5ed1967` before final
validation. Main already contains the Claude thinking-block completion and
history/scheduler changes from PR 568. This change retains them.

## Confirmed causes

- OpenCode 1.18.3 streams reasoning with `field: "text"`, the same field as
  answer text. Bridge classified it as prose instead of consulting the part.
  Cumulative thinking snapshots were also appended as deltas.
- OpenCode repeats busy/model-step notifications within one submitted turn.
  Treating each as turn.started changes the active turn identity and grouping.
- Stop's global boolean and deferred request could follow a selected-chat switch.
  The native command also ran blocking provider interruption on an async worker.
- Visited forest snapshots stayed in an unlimited Map. Live event limits counted
  visible text but omitted nested tool/provider payloads. Cursor's second event
  channel was unbounded despite its upstream count cap.
- Top-level idle provider processes had no shared retention policy.

## Changes and budgets

| Retention | Policy |
| --- | --- |
| Visited chat histories | LRU, 4 chats and 16 MiB estimated serialized UTF-16 size |
| Shared live event tail | Existing text/count limits plus 8 MiB including nested payloads |
| Cursor reader backlog | Existing shared frame-queue policy: 2,048 items / 8 MiB; shed recoverable text/thinking deltas first; backpressure terminal events |
| Eligible idle chat processes | At most 2 retained; expire after 120 seconds; one release per maintenance tick |

The process policy applies across harnesses. Active turns, approvals, workers,
queued input, and lifecycle operations are excluded. Submission holds a shared
lease; reclamation uses a nonblocking exclusive lease. Shutdown holds neither
the database/map locks nor the input lease. History/provider IDs survive;
continuation uses the existing native resume or projected restoration path.
Cursor does not advertise native resume, so its restoration can lose provider-only
context; the transcript is preserved. The policy trades idle warmth for RAM.

Composer Stop commits cancellation, closes the reader gate, and publishes the
terminal event before process disposal. It does not await an HTTP abort or start
a summarization turn. Other chats are unchanged. The UI keeps deferred stops
keyed by their original session and reports failures.

## Evidence and limits

`src/transcript/fixtures/opencode-wire.json` was captured from installed OpenCode
1.18.3 using an isolated home/config directory and a loopback fake provider. Its
IDs are anonymized and paths removed. Rust checks the captured wire against the
expected normalized events; TypeScript reduces those same events and checks
thought completion before answer output and replay without duplicate prose.

In that single cold run, first thinking arrived about 737 ms after prompt
submission. Plugin/catalog initialization occurred before the first model step
(about 684 ms); subsequent chunks followed the fake provider's 50 ms cadence.
This is not a production latency percentile or proof that HTTP causes the wait.
No paid model request was required for that capture.

Regression coverage includes deferred/concurrent Stop ownership, backend
cancellation isolation, cached-history eviction including nested data, Cursor
backpressure with 10,000 deltas and terminal recovery, and idle reclamation with
active/approval/submission exclusions and resume ID preservation.

These are retention budgets, not a hard cap on RSS of active vendor processes.
One oversized terminal frame is admitted alone by the shared queue to avoid
losing durable content. Active generations, provider latency, and slow Git/DB
contention tracked by issue 545 remain outside a zero-latency guarantee. Abrupt
Stop can prevent provider-only, not-yet-finalized content from being persisted.

## Acceptance and manual checks

- A reasoning part streamed through `field: text` remains thinking, closes before
  the answer, and replays once. Repeated busy/step frames do not create turns.
- Stop A while A is launching; switch to B. Only A receives interruption when its
  launch is acknowledged. Stop A and B independently; a failed request is visible.
- A committed stopped state clears the indicator before process disposal replies.
  Late frames cannot restore a stopped turn; the next resume has no stale marker.
- Visit more than four chats and feed nested large event payloads. Retained caches
  stay within the documented budgets; durable history remains reloadable.
- Keep active and approval-waiting processes beside idle processes. Only eligible
  idle roots retire, and provider resume IDs survive.
- In `bun run dev`, open the demo orchestrator, expand command output, type a
  draft, and switch chats. The selected header and transcript must agree; another
  chat must not inherit Stop. Scroll-follow and thinking expansion are also
  covered by the existing conversation interaction suites.

Validation uses `bun run build` and `bun run test`, plus the captured OpenCode
wire replay. Browser smoke checks use the mock-backed Vite app; they are not a
native WebKit latency benchmark. Real paid-provider end-to-end sessions for all
four harnesses were not run. The recorded local OpenCode capture is the live
transport check.

## Final verification

- `bun run build`: passed on merged main `5ed1967`.
- `bun run test`: passed (2,046 frontend tests; 2,569 Rust tests across the
  workspace, including 2,251 bridge-core tests; release and Claude sidecar checks
  also passed). Existing explicitly ignored tests remain ignored.
- Used a private Cargo target with debug symbols disabled, Rust test concurrency
  eight, and Vitest fork concurrency two. A shared target initially picked up an
  incompatible protocol artifact from another checkout; isolation resolved it.
  Overlapping unrestricted frontend runs exposed existing syntax-highlighter
  timeouts; the bounded final full run passed without changing those tests.
- Browser mock smoke: expanded command output, typed while it remained open,
  stopped the active chat, and switched to another chat with its own transcript
  and no inherited Stop indicator.
