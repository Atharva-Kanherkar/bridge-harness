# codex/pr-127-adversarial — Test Contract

## Functional Behavior

- A protocol 0.5 server continues accepting every params document that was valid under the published 0.4 schemas.
- Methods first contracted in 0.5 reject unknown top-level params fields.
- The `BrowserActionRequest` mirror test fails to compile or test when the core DTO adds, removes, renames, or retypes any field.
- The command-signature drift gate detects incompatible Rust argument-type changes, while allowing intentional contract narrowing from `String` to `ApprovalDecision` and `BrowserPermission`.

## Unit Tests

- Existing `bridge-protocol` artifact and payload tests pass.
- Add a compatibility test proving all previously published 0.4 params schemas remain open to additional properties.
- Add an invariant test proving all params schemas first introduced in 0.5 set `additionalProperties: false`.
- Strengthen `protocol_mirror::browser_payloads_mirror_core` to compare a fully populated core `BrowserActionRequest` with its protocol mirror.
- Add signature-parser fixtures covering `Option<u32>`, `i64`, `u16`, `Vec<String>`, core DTOs, and intentional string-to-enum narrowing.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- Generated JSON Schemas and TypeScript artifacts exactly match the Rust contract.

## Smoke Tests

- `bun run build` passes.
- `bun run check` passes.

## E2E Tests

N/A — this PR changes generated protocol contracts and compile/test-time drift gates, not a user-facing UI flow.

## Manual / cURL Tests

N/A — the daemon transport that consumes these schemas lands in a later PR. Compatibility is verified directly against the checked-in 0.4 schema artifacts in unit tests.
