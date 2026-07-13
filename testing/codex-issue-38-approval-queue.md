# Issue 38 Contract: Human approval must not silently consume queue TTL

- A queued worker whose parent or any session ancestor is `waiting` transitions durably to `blocked_on_human` before queue expiry is evaluated.
- `blocked_on_human` is not dispatchable and cannot expire while approval remains unresolved.
- When no ancestor is waiting, the item returns to `queued` and its expiry advances by the full blocked duration, preserving its remaining TTL.
- Parent cancellation terminates queued, dispatching, and human-blocked work rather than releasing it.
- Block and release transitions emit durable reason events, and the Deck explanation names the human dependency instead of showing a generic capacity wait.
- Existing queue rows migrate without losing expiry or dispatch state.

## Local refund checkpoint

Verify direct-parent and transitive-ancestor blocking, expiry pause/release arithmetic, cancellation, persistence/migration, presentation, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
