# `/voice` composer dictation contract

Issue: #651

## Scope

The currently wired transport is Codex-only dictation for an already-running direct chat.
Bridge captures microphone audio, streams bounded PCM chunks through the existing
Codex app-server process, and writes only the user-role transcript into the
composer. It never submits the draft automatically.

The user approved opt-in local, harness-independent dictation on 2026-09-21;
delivery is tracked in `docs/plans/voice-dictation-rework.md`. The independent
daemon service and test-only fake engine now exist; the native engine and its
setup flow are not yet connected. Claude's internal speech endpoint and implicit
cross-provider fallback remain excluded. Other harnesses show a disabled mic with
an unavailable reason, rather than hiding the control.

## Delivery phases

1. **Protocol foundation — complete.** Add the versioned voice messages,
   capability response, transient transcript event, generated TypeScript, and
   schema/registry coverage.
2. **Codex transport — implemented; runtime validation pending.** Gate on the
   experimental schema and route realtime requests/notifications. Provider request
   correlation and confirmed dictation-only behavior remain unverified.
3. **Capture and composer — safety rework implemented.** Bounded client queues,
   preview-only partials, snapshot-checked final insertion, and mic controls.
   AudioWorklet/continuous resampling and packaged capture validation remain pending.
4. **Lifecycle hardening — reopened.** Client operation ownership, terminal cleanup,
   deadlines, and draft-edit protection have regression coverage. Backend expiry
   and late events across successive takes on the same provider thread still need
   work. Previous passing tests did not establish end-to-end safety.
5. **Live validation and delivery — pending.** Exercise an authenticated Codex
   session with macOS microphone permission, collect acceptance evidence, and
   prepare the PR.
6. **Independent local provider — foundation implemented.** Native evaluation is
   recorded separately. The daemon-owned provider/stream boundary accepts fresh
   draft owners, without a coding session, database mutation, adapter launch, or
   credential lookup. A test-only fake verifies the contract. The production
   service reports `needsSetup`; real engine integration and setup remain pending.

## Protocol

- Protocol 1.18 adds `voice/capabilities`, `voice/start`, `voice/append`,
  `voice/stop`, and `voice/cancel`.
- Protocol 1.19 separates required `ownerKey` from optional `sessionId`, types
  provider IDs, exposes `ready`/`needsSetup`/`unsupported`/`failed` capability
  states and processing location, and adds replacement `partial` hypotheses.
  Old clients/daemons are rejected in both directions. Transcript payloads now
  have generated schemas/TypeScript rather than a hand-maintained frontend shape.
- Every live operation after start is addressed by an opaque `voiceSessionId`.
- Chunks carry a zero-based, strictly increasing sequence and are bounded both
  per chunk and per session.
- `voice-transcript` is transient. Every payload carries the draft owner key,
  optional coding-session id, voice session id, provider, kind, and text/error
  fields appropriate to the kind. Only a `partial` replaces the current
  provisional hypothesis; a legacy `delta` appends to it.
- The client rejects frames for cancelled, completed, replaced, or wrong-provider
  voice IDs. Backend correlation of late frames across reuse of a provider thread
  is not yet proven; this is a release blocker for the experimental Codex path.

## Provider behavior

- Capabilities can be probed without any coding session. They never install/load
  a local model, select a provider, or fall back to another provider.
- Local takes use one worker and a two-frame bounded input channel, validate
  sequence/PCM/chunk/total limits, and hold their busy slot until engine cleanup
  finishes. Startup and RPC waits are bounded; idle takes expire actively.
  Late replies after cancellation or deadline cannot emit a fresh take's text.
- Future real-engine implementations must honor the cancellation token and
  supervise/kill/reap their helper process. A non-cooperative engine currently
  remains busy until it returns; the service deliberately cannot start additional
  workers around a stuck one. This is not yet a hard-kill native implementation.
- Engine failures expose fixed diagnostic messages, not raw engine errors,
  audio payloads, or transcript text. Fake providers are available only in tests,
  never through a runtime setting or a shipped fallback.

- Codex is offered only when the installed experimental schema contains the
  complete realtime request surface and the live app-server was initialized with
  `experimentalApi: true`.
- Start uses the active chat's existing Codex app-server connection. It does not
  launch a second provider process or create a Bridge turn.
- Bridge requests text output with client-managed handoffs and no startup
  context, so dictation does not automatically hand transcript text to Codex.
- Only user-role transcript deltas/finals reach the composer event.
- Provider error/closed notifications release client capture and terminate its
  matching take. Successful start/stop writes alone do not establish readiness
  or finalization; the client waits for notifications with bounded deadlines.

## Capture and composer behavior

- The mic is visible with a reason when unavailable. Only an idle, live Codex
  direct chat currently enables capture; Retry refreshes capabilities/events.
- Click toggles capture; a hold of at least 300 ms stops on release. Space is
  hold-to-talk, Enter finishes without sending, and Escape cancels. `/voice`
  invokes the same controller.
- Audio is downmixed/resampled to 16 kHz mono PCM s16le before transport.
- Partial/final utterances render in a separate preview. Terminal completion
  inserts finalized text at the captured selection only if draft owner, revision,
  and text are unchanged. No partial transcript edits the draft. Surrounding
  whitespace is preserved; an external mutation invalidates insertion.
- Editing, attachments, and Send are blocked during starting/recording/stopping;
  cancellation remains available. Audio buffering is bounded, and client startup,
  delivery, maximum duration, and stop deadlines prevent indefinite active state.
- Permission denial, unavailable input, provider refusal, and transport failure
  leave the pre-existing draft intact and show an error.
- Switching chats or unmounting cancels capture and releases media tracks.

## Required checks

- Protocol registry, payload-schema, generated-artifact, and stale-daemon tests.
- Core tests for capability gating, ordering, chunk/session bounds, cancellation,
  late frames, wrong roles, and finalization.
- Codex adapter tests for schema detection and exact realtime request shapes.
- Composer tests for mic states and draft preservation.
- Frontend capture tests for PCM conversion/resampling and cleanup.
- Independent service tests: fresh draft without a coding session, explicit
  setup state/no fallback, sequential/revisable hypotheses, startup/append
  timeout with retained engine ownership, active expiry, drop cleanup, stale
  take IDs, malformed/oversized/over-budget PCM, and queue backpressure.
- `bun run build` and `bun run test` are green.
- Packaged macOS configuration contains the microphone usage description and
  audio-input entitlement; no provider credential is required by packaging
  smoke tests.
