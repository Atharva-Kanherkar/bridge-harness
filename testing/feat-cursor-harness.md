# feat/cursor-harness — Test Contract

Issue #368, second slice. Stacked on feat/acp-client-layer, which shipped the shared
layer this uses. Locked before implementation.

## The shape of the problem

The ACP client layer exists and is tested, and nothing reaches it. Cursor ships an ACP
server in its own CLI, which makes it the first harness Bridge can adopt without
writing a provider adapter from scratch: discovery, launch, authentication and
availability are Cursor-specific, and everything after the handshake is already shared.

Cursor is a vendor CLI the user installs, not a runtime Bridge fetches, so it belongs
beside Claude, Codex and OpenCode rather than in the managed-install catalog, whose
recipes all describe something to download. Discovery follows the same shape those
three already use.

Two properties of the vendor make naive discovery wrong, and both are the reason this
slice is more than a registration:

- The ACP subcommand is not advertised. It does not appear in help output, so presence
  cannot be established by reading help.
- A build without ACP support does not reject the subcommand. It treats it as a prompt
  and starts an interactive terminal session, writing terminal control output where a
  client expects protocol. A supervisor that waits for a handshake waits forever.

## Functional Behavior

### Discovery

- Bridge looks for the vendor's own executable under both names it ships under,
  preferring the unambiguous one. The bare name is shared with an unrelated vendor's
  CLI, so finding it is not on its own evidence that Cursor is installed.
- Nothing is installed, downloaded, or modified. An absent executable makes the harness
  unavailable with a reason that names what was looked for, exactly as a missing
  provider CLI does today.
- Discovery reports a version when it can read one. A version it cannot parse makes the
  harness unavailable with that stated, rather than being treated as new enough.
- Availability is resolved without launching a session, and a harness that is
  unavailable never appears selectable.

### Confirming the protocol, not assuming it

- Before the harness is offered, Bridge confirms the executable actually speaks the
  protocol by completing a handshake against it under a timeout, and treats output that
  is not a protocol message as a failed probe rather than as noise to skip past.
- A probe that times out, exits, or answers with anything else leaves the harness
  unavailable with an actionable reason, and reaps whatever it started.
- The probe result is cached with the version it was taken against, so selecting the
  harness does not re-probe on every read, and a changed version invalidates it.

### Authentication stays the vendor's

- Bridge never reads, copies, or stores vendor credentials, profile data, or cookies.
  An existing vendor login is used as it stands.
- Where the vendor advertises an authentication method, Bridge uses the advertised one
  rather than a hardcoded name, and treats an unadvertised method as unavailable.
- An explicitly configured key is passed only in the launched process environment. It
  is absent from logs, from failure context, from the adapter descriptor, and from
  anything durable.
- An authentication failure is reported as needing sign-in with the vendor's own
  command named, not as a crash and not as a generic unavailability.

### Capabilities are read from the session, not declared by Bridge

- Models and modes come from what the session reports, not from a list compiled into
  Bridge. A build that offers different models offers them without a Bridge change.
- Model identifiers are echoed back exactly as received. Bridge never reconstructs one
  from a label or a command-line spelling; the identifier namespaces differ and a
  reconstructed one is rejected by the vendor for every model, which reads to a user
  like a billing problem rather than a client bug.
- Native resume is offered only where the session advertises it. Where the vendor
  advertises history replay but not resumption, Bridge reports checkpoint handoff and
  does not claim resume it does not have.
- A capability the session did not advertise is never exercised.

### Vendor-specific traffic is answered, never ignored

- The vendor issues requests outside the protocol's own vocabulary, and some of them
  block until answered. Any request Bridge does not handle is answered with an error;
  none is left unanswered, because an unanswered blocking request stalls the agent
  indefinitely.
- Vendor notifications that Bridge has no use for are recorded as unknown events rather
  than dropped silently.
- Permission options are echoed back by the identifier the vendor offered. Bridge never
  substitutes its own vocabulary, and the identifiers are not assumed to match the
  protocol's own naming.

### Policy is unchanged

- Permission requests enter the existing approval queue. Nothing here can auto-grant,
  widen a write scope, or bypass sandboxing, and the harness runs under the same
  isolation every other harness does.
- The harness is a candidate like any other. Nothing in this slice pins it, excludes
  it, or gives it standing the policy engine did not already grant.

## Determinism

No test requires Cursor to be installed, contacts a network, or sleeps. Discovery is
tested against fixture executables on a controlled search path, including the
ambiguous-name case and the case where a build answers the subcommand with terminal
output instead of protocol. Session behavior is already covered by the shared layer's
tests and is not re-tested here.

## Out of Scope

- Any change to the shared ACP layer beyond what registering a harness requires.
- Retiring or reworking the Claude, Codex and OpenCode adapters.
- Vendor features reached outside the protocol.
- Every UI surface: the harness appears through the existing adapter descriptor list.
