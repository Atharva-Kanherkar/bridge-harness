# Worker result pipeline and status vocabulary (issue #572, slices 6–7)

## Reading a result (slice 6)
- A worker's envelope is found by scanning back through its recent assistant messages for one that carries a `bridge-worker-result` block, newest first. A valid result followed by chatter is a result, not a repair turn.
- An envelope whose body quotes a fenced snippet is recovered by matching braces rather than fences, so a diff or a shell command inside `decisions` does not truncate it. Braces and backticks inside JSON strings are contents, not structure.
- `protocol_invalid` is Bridge's verdict about an envelope it could not read. A worker declaring it about its own readable envelope is read as `failed`, and the normalization is recorded.
- A worker recovered after a restart reaches its parent's transcript, not just its own runtime row.

## Naming a failure (slices 6–7)
- `FailureClass` carries `stalled` as a class of its own, decided from Bridge's observation that the worker went silent — never from the summary Bridge itself wrote, which contains the word "timeout" and made a hung worker look transient and retryable.
- A stalled worker is declined for retry: its entire evidence is that it stopped producing evidence.
- The verdict is persisted on `worker_runtime.failure_class` and travels on the protocol record, so every surface reads one classification instead of re-deriving it. No surface pattern-matches a Rust format string.

## Showing a worker (slice 7)
- Terminal is terminal: a worker whose session has ended does not pulse, and its clock stops.
- A cancellation renders as a neutral decision, not a destructive failure.
- `protocol_invalid` has its own label rather than falling through to a generic one.

Verification: parse tests for result-then-chatter, quoted fences, braces in strings and self-declared `protocol_invalid`; behavioural tests that the backwards scan finds an earlier envelope, that the newest message is still used when no envelope exists, that an observed stall classifies `stalled` and is not retried, and that stall *wording* alone does not. Run `bun run build` and `bun run test` before opening the PR.
