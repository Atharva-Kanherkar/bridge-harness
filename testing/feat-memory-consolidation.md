# feat/memory-consolidation — Test Contract

Issue #214, the Phase D remainder. Locked before implementation. Stacked on
feat/bounded-evaluator, so the schema starts at 41 and this slice takes 42.

## The shape of the problem

The ledger records that a memory was replaced but not when it stopped being true.
`supersede` flips the predecessor to superseded and links the chain, and that is the
whole temporal story: there is no point in time at which the old fact ceased to hold,
so a record cannot answer what the user believed last month. There is no expiry, so a
fact learned once is active forever whether or not it was ever true again. There is no
conflict group, so two records that contradict each other are simply two active
records, and the packet may inject both. And there is no way to merge: a scope
accumulates until retrieval quality degrades, with nothing that ever reduces it.

Every one of those is named in the issue's record contract and none of them exists.

This slice adds the deterministic half in full, and one bounded LLM job that may only
propose. Nothing here can grant a permission, widen a scope, or skip an approval.

## Functional Behavior

### Validity is an interval, not a flag

- A record carries the instant its claim began and, once it stops holding, the instant
  it stopped. An active record's end is open.
- Superseding closes the predecessor's interval exactly where the successor's opens.
  There is no gap in which neither held, and no overlap in which both did.
- A closed record stays queryable as history. Nothing in this slice deletes a row, and
  the existing tombstone remains the only path to removal.
- Reading a scope as of an instant returns exactly the records whose interval contains
  it. As of now, that is the same set the ledger returns today, so nothing that reads
  the ledger changes behavior.

### Expiry is deterministic and reversible only by the user

- A record may carry an expiry. Passing it moves the record to a new status, distinct
  from superseded and from deleted, and closes its interval at the expiry rather than
  at the moment the sweep happened to run.
- Expiry is applied by a sweep that takes the current time as an argument. It never
  reads a clock inside a query, so a test pins exact boundaries: a record expiring
  exactly at the sweep instant is expired, one expiring after it is not.
- An expired record leaves the retrieval index, so it cannot reach a packet. The
  existing index triggers already key on active status; this must not need a second
  mechanism.
- Expiry is a lifecycle transition, not a deletion: the body, provenance and interval
  all survive, and the user can see why a record stopped applying.
- A record with no expiry never expires. Explicit user saves default to no expiry.

### Contradiction is a group, not a race

- Records that make competing claims about the same subject share a conflict group.
- At most one member of a group is active at a time. Activating a member closes the
  interval of whichever member was active, exactly as supersession does.
- A packet built from a scope containing a conflict group contains at most one member
  of that group, so two contradictory facts can never be injected together.
- A group with no active member is a legible state, not an error: every member expired
  or was rejected, and the scope simply has no answer for that subject.

### A bounded scope refuses rather than evicts

- A scope carries a budget. A write that would exceed it fails, naming the budget and
  what is currently held, and the scope is left exactly as it was.
- The refusal reaches the writer that caused it. An explicit user save is refused with
  a message the user can act on; a proposal that would overflow is refused before it is
  stored, and the run records that it was refused rather than silently dropping it.
- Nothing evicts on overflow. Silently discarding the oldest record is the failure mode
  where a user cannot tell whether memory was full, trimmed, or never written.

### Consolidation proposes; the gate decides

- The consolidation job runs against a trait seam, exactly as extraction does. No test
  performs a model call.
- It sees the scope's active records and nothing else: no session entries, no
  transcript, no other scope.
- It answers with a closed vocabulary of operations against records that already exist,
  each naming a target. Anything outside that vocabulary, a target that is not in the
  scope, a target that is not active, or an operation the settings do not permit is
  refused and counted, and the rest of the batch still applies.
- A merge is expressed as a supersession of every record it replaces, so provenance and
  the interval chain survive the merge and every source stays reachable.
- Removal is opt-in. With removal disabled, an operation asking for it is refused
  rather than downgraded to something else.
- A proposal can only produce a record the user could have produced. It cannot set a
  status, a provenance, a scope, an interval, or an expiry the gate did not derive.

### It runs when the session is quiet, not while it is talking

- A run is enqueued per scope, debounced: a new turn in that scope while a run is
  pending replaces it rather than queueing a second, so consolidation never runs
  against a conversation still in progress.
- Enqueueing is gated on remaining budget. With the budget exhausted the run settles
  as skipped, not failed, and no model call is made.
- Runs are leased with an expiry, claimed at most once, and settle exactly once, with
  observed tokens and spend recorded, as extraction runs already do.
- Turning consolidation off settles queued runs rather than executing them.

## Determinism

Every instant is passed in. No test sleeps, reads the wall clock to make an assertion,
or depends on sweep ordering. Lease, claim, expiry, and interval boundaries are all
asserted against explicit timestamps.

## Out of Scope

- Retrieval ranking. The packet's selection rules are unchanged apart from honoring the
  new statuses and the conflict group.
- A replay benchmark corpus. The deterministic fixtures here cover the contract; a
  scored benchmark is its own piece of work.
- Routing consolidation through the learning router. It runs on the configured harness
  and model, as extraction does.
- Every UI surface. The wire methods this adds are settings and read only.
