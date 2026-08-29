# fix/cursor-approval-lifecycle — Test Contract

Review follow-ups to the merged ACP client layer and Cursor harness. Written
against the findings rather than before them, because these are fixes to
shipped behavior rather than a new slice.

## The shape of the problem

Three separate gaps, all in the seam between the shared ACP client and the rest
of Bridge.

A permission Bridge answers on the agent's behalf leaves the card that raised
it pending forever. `AcpSession::cancel` answers every parked responder with a
cancelled outcome, so the agent is unblocked, and it pushes `approval.settled`
— but nothing consumes that event. Pending state is decided entirely by the
absence of an `approval.resolved` entry, so the card keeps its buttons and each
one now fails with "no longer outstanding". The existing question path already
had this exact shape, keyed on the OpenCode question method.

Cursor reached the adapter registry, the backend table, the compatibility
contract and `BUILTIN_HARNESS_IDS`, while `agent_config` still listed four
harnesses. The harness was therefore runnable but unconfigurable: no row to set
a default model, an effort or a system prompt on, no entry in the Settings
list, and orchestrator selection skipped it against a hardcoded three-name
match.

The event queue counted evictions that nothing read.

## Functional Behavior

### A settled permission retires its card

- An `approval.settled` carrying a cancelled outcome resolves the matching
  `approval.requested` entry and unblocks the session or worker, the same three
  writes the question path performs inline.
- Only the cancelled outcome settles here. A selected outcome came from
  `resolve_approval`, which writes the resolution itself; resolving twice would
  put two markers in one transcript.
- Delivering the same settlement twice is one resolution: the lookup only ever
  finds a request nothing has resolved yet.
- Request ids compare as text on both sides. A provider question carries a
  string id and an agent-protocol permission carries a number, and an integer
  never compares equal to a string in SQLite — the numeric half would silently
  match nothing.

### Every bespoke adapter is configurable

- A harness Bridge registers an adapter for has a configuration row: a default
  model, an effort and a system prompt can be set on it, and it appears in the
  Settings runtime list.
- Orchestrator selection asks the registry which harnesses it runs rather than
  restating a list. A configured harness with no adapter behind it still
  resolves nothing, and an adapter that is registered but unavailable still
  falls through on model resolution exactly as before.
- `shell` is excluded by intent: a terminal is not an agent anyone points a
  prompt at.

### A failed handshake cannot wedge on a wedged transport

- The failure path joins the connection thread only when it reported that it
  finished. The wait is bounded precisely because the transport may be stuck,
  and joining one that never answered would block forever on the condition the
  timeout exists to survive.

### One probe at a time

- The background discovery pass and a caller that arrived before it landed take
  the same gate, and whoever waits re-reads the cache first. Two callers never
  spawn two vendor processes to ask the same question.

### A shortened stream is reported

- Queue depth and eviction count are answered on the adapter registry's
  existing metrics seam. Byte fields stay zero because this queue bounds by
  item count, not bytes; a fabricated size would be worse than none.
- Eviction prefers streaming deltas and falls back to the oldest durable event
  when the queue holds nothing transient — the newest event is kept rather than
  refused, and the drop is counted.

## Determinism

No test spawns a vendor binary for the approval or configuration paths. The
approval test drives `handle_agent_value` with an encoded normalized event, the
same shape the Cursor pump writes and the reader reads back. Queue behavior is
asserted against `EventQueue` directly.

## Out of Scope

- Whether registering Cursor should probe the vendor CLI on every app launch.
  That is a cost and telemetry decision, not a defect, and it belongs to
  whoever owns the vendor relationship.
- Capturing the configured key at launch so a key changed mid-session is still
  the one redacted at report time.
- A test for shutdown while a prompt is in flight.
