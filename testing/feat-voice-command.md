# `/voice` composer dictation contract

Issue: #651

## Scope

The composer selects opt-in, harness-independent local dictation. Bridge captures
microphone audio, streams bounded PCM chunks to a supervised sherpa-onnx helper,
and inserts finalized text into the unchanged draft. It never submits the draft
or creates a coding turn automatically.

Settings provides explicit engine/model download, progress, retry and removal.
Downloads are pinned by size and checksum and activated atomically. Capability
probes never download or load a model. Production setup currently supports Apple
silicon Macs and an English speech model.

The Codex realtime transport remains disabled and is not selected by the
composer. Authenticated dictation-only behavior and late-event correlation across
successive takes on one provider thread remain unverified. Claude's internal
speech endpoint and implicit cross-provider fallback remain excluded.

## Delivery status

- Typed protocol, local setup/removal, supervised native helper, AudioWorklet
  capture and composer safety are implemented.
- Automated checks cover client ownership, preview/final insertion, PCM limits,
  cancellation, helper cleanup, download verification and generated artifacts.
- Live microphone validation on the exact packaged build remains pending.
- Authenticated Codex validation and safe take correlation remain pending; keep it disabled.

## Protocol

- Protocol 1.21 includes draft-owned dictation, local setup/status/removal,
  typed provider capabilities and transient transcript events. It also retains
  main's attribution, chat search and context-window methods. Pre-voice clients
  and daemons are rejected in both directions.
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
- The production engine observes cancellation through a supervised helper,
  which is killed and reaped before the busy slot is released. Bounded reader
  queues are disconnected before joining their threads, and cleanup does not
  write to a potentially blocked helper. A non-cooperative test provider remains
  busy until it returns; additional workers cannot bypass that ownership.
- Model removal is serialized with recording admission and refuses an active
  take or installation. Successful setup refreshes composer availability,
  including an immediate ready response.
- Engine failures expose fixed diagnostic messages, not raw engine errors,
  audio payloads, or transcript text. Fake providers are available only in tests,
  never through a runtime setting or a shipped fallback.

- Codex remains disabled until individual recordings can be correlated safely.
  Schema support alone must not enable the transport.
- The transport also requires that the installed experimental schema contains the
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

- The mic is visible with a reason when unavailable. An idle composer with an
  installed local engine enables capture regardless of coding harness. Retry
  refreshes capabilities/events; Settings exposes model setup.
- Click toggles capture; a hold of at least 300 ms stops on release. Space is
  hold-to-talk, Enter finishes without sending, and Escape cancels. `/voice`
  invokes the same controller.
- Audio is captured through a packaged AudioWorklet, with a compatibility
  fallback, continuous 16 kHz PCM s16le conversion and acknowledged tail flushing.
  A suspended audio context is resumed before startup completes.
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
