# feat/acp-client-layer — Test Contract

Issue #368, first slice: the shared layer, no vendor. #367 (Grok Build) reuses it.
Locked before implementation.

## The shape of the problem

Bridge already reads the ACP registry (`acp_registry.rs`) and already has a frame
vocabulary for the protocol: `AcpDriver` in `agent_integration.rs` spells a prompt, a
resume, and a permission decision. What it has never had is a client that speaks to a
real agent process. `runtime.rs` builds `IntegrationRegistry::empty()`, and the only
`Transport` is `ScriptedTransport` in the tests.

Writing a JSON-RPC layer by hand would be the wrong answer. The protocol's own Rust
crate is published, is `Send` throughout, pulls no async runtime of its own, and already
owns child-process supervision: process-group leadership with a group-wide kill on drop,
continuously drained and bounded stderr, and a shutdown grace period. Bridge duplicates
the first two by hand in `adapters.rs`. This slice adopts the crate and wraps it in one
Bridge-shaped module.

Two things the existing frame vocabulary gets wrong, both fixed here because a live
agent will not tolerate either:

- A permission decision is not a decision string. The response is an outcome object,
  either a selection carrying back one of the `optionId` values the agent offered, or a
  cancellation. `AcpDriver::decision_frame` currently emits a bare outcome string.
- An agent may send vendor-specific requests that block until answered. An unknown
  method that is silently ignored wedges the agent. It has to be refused.

## Functional Behavior

### Connection

- One supervised child per session, connected over the protocol crate, driven from a
  dedicated thread. `bridge-core` gains no async runtime: the connection future is
  driven to completion on that thread, matching the existing thread-per-adapter shape.
- Initialization negotiates protocol version 1 and records what the agent advertised:
  load-session support, prompt capabilities, session sub-capabilities, and the offered
  authentication methods. Draft protocol versions are not requested.
- An advertised capability is recorded, never assumed. A method whose capability is
  absent is not called, and asking for one is a Bridge-side error rather than a wire
  round-trip.
- Session sub-capabilities are presence-signalled empty objects. Support is decided by
  presence, not by truthiness, and a capability object that fails to parse degrades to
  absent rather than failing the whole handshake.
- Initialization is bounded by a timeout. An executable that accepts the connection and
  then says nothing, or answers with something that is not a protocol message, fails
  with the offending output captured, and the child is reaped rather than left running.

### Turns and streaming

- A prompt is one text content block against the live session id. Streamed updates are
  translated into Bridge's normalized events: assistant message chunks, thoughts, tool
  calls and their updates, plans, mode changes, available-command lists, and usage.
- Tool-call kind and status are mapped onto Bridge's existing vocabulary, and an
  unrecognized kind or update variant is reported as an unknown event rather than
  dropped or panicked on. The protocol's enums are explicitly open, so the mapping is
  written as a non-exhaustive match with a named fallback.
- A permission request becomes Bridge's approval event carrying the offered options.
  The answer echoes back one of the offered option ids verbatim; Bridge never invents an
  option id and never substitutes its own allow/deny vocabulary on the wire.
- An unknown or unhandled incoming method is answered with a method-not-found error, not
  ignored. A test drives a fake agent that issues an unrecognized blocking request and
  asserts the agent receives an error response rather than waiting.

### Cancellation

- Cancelling sends the protocol cancel and does not kill the process.
- Every permission request still outstanding is answered with a cancelled outcome.
- Updates that arrive after the cancel are still accepted until the turn's own response
  lands; the turn ends on a cancelled stop reason, which is a normal completion and not
  an error.
- Stop reasons are surfaced distinctly: an end of turn, a token limit, a turn-request
  limit, a refusal, and a cancellation are not collapsed into one "finished".

### Resume

- Bridge asks to reload a prior provider session only when the agent advertised that it
  can, and asks to reconnect without replay only when the agent advertised that
  separately. Neither is called speculatively.
- History replay arrives as updates **before** the reload call returns, so routing is
  installed before the request is published; no replayed update is lost to a late
  subscription.
- Bridge's session forest is the owner of conversation history. Replayed entries are
  reconciled against what the forest already holds and never appended a second time. A
  reload of a session the forest already has produces no new forest entries.
- A reload of an unknown provider session fails with a legible error naming the id.

### Process lifecycle

- Shutdown closes the input stream, allows a grace period, and then terminates the whole
  process group. It is idempotent, and no child or grandchild survives it. A test
  asserts the pid is gone after shutdown and after a failed initialization.
- stderr is drained continuously into a bounded rolling tail, so a chatty agent cannot
  deadlock on a full pipe. On exit the failure context reports the exit status together
  with that tail, bounded, never a transcript.
- A child that dies mid-turn ends the update stream cleanly rather than erroring
  forever, and the turn resolves with the failure context attached.

## Determinism

Tests drive the protocol crate's in-process duplex channel — two connected endpoints,
no subprocess, no sockets, no sleeps — so the whole mapping, permission, cancellation,
and unknown-method surface is exercised deterministically. Process lifecycle tests use a
tiny fake agent that is part of this repo and are the only tests that spawn anything.
No test contacts a network or requires a vendor binary.

## Out of Scope

- Discovering, launching, authenticating, or registering Cursor. That is the next slice.
- Retiring or reworking the existing Claude, Codex, and OpenCode adapters. They stay
  exactly as they are.
- Draft protocol features: forking, compaction, provider lists, next-edit suggestions.
- Every UI surface.
