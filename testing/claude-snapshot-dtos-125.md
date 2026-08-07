# claude/snapshot-dtos-125 — Test Contract

## Functional Behavior

- Existing health responses retain the `telemetry_database` and `snapshot_directory` JSON keys accepted by protocol 0.6 clients.
- `BridgeMethodResults` includes every method in the method registry; deferred methods resolve to `unknown` rather than disappearing.
- JSON-schema objects whose only shape is `additionalProperties` generate typed TypeScript records.
- Root and nested schemas generate exactly one TypeScript declaration per exported name.
- Snapshot integer values exposed to JavaScript reject values outside JavaScript's safe integer range.
- Session-status mirror coverage is compile-time exhaustive when a status variant is added.

## Unit Tests

- TypeScript generator tests cover typed maps, the complete method-result map, and duplicate-declaration prevention.
- Protocol message tests cover safe signed and unsigned integer boundaries and out-of-range rejection.
- Health serialization tests assert the legacy snake_case field names.
- Mirror tests exercise every session-status mapping through an exhaustive conversion function.

## Integration / Functional Tests

- Regenerate all protocol schemas and `src/protocol/generated/protocol.ts` from the Rust source of truth.
- Run the bridge protocol and core Rust test suites against the regenerated artifacts.
- Run the frontend TypeScript build against the generated declarations.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully after preparing the daemon sidecar.

## E2E Tests

N/A — these changes affect protocol serialization and generated type artifacts, not a user-facing journey.

## Manual / cURL Tests

N/A — automated serialization, artifact freshness, build, and test checks cover the changed contract surface.
