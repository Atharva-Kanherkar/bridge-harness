# Issue 35 Contract: Versioned persisted semantic events

- `session_entries` stores an explicit semantic schema version separately from provider payloads and checkpoint-specific versions.
- Existing entries migrate to semantic schema v1; every new production append writes semantic schema v2.
- Rust restoration/context projection and the Deck conversation projector accept current v2 and N-1 v1 entries with equivalent semantics.
- Unsupported future semantic versions fail closed in Rust traversal rather than being silently misread.
- Fresh and upgraded databases retain identical schemas, and migration remains idempotent with a pre-migration backup.

## Local refund checkpoint

Verify fresh/upgraded schema parity, v1 migration, v2 append, v1/v2 projection parity, future-version rejection, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
