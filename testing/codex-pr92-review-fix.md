# codex/pr92-review-fix — Test Contract

## Functional Behavior

- Read-only Codex and Claude workers start inside an OS-enforced sandbox before provider code runs.
- The assigned workspace is readable but rejects tracked-file edits and untracked-file creation.
- Each read-only worker receives a separate writable output directory outside the workspace.
- Network access is denied by default and can be enabled only when both the typed delegation request and Bridge policy authorize it.
- Writable output paths are relative, bounded, traversal-safe, and confined to the assigned output directory.
- Shared, isolated, and full-write worker behavior remains unchanged.
- Unsupported platforms or missing isolation primitives fail closed with a clear error.
- Sandbox mode, network authority, writable paths, launch failures, and cleanup are auditable.
- Sandbox output is cleaned up on normal completion, cancellation, failure, and process shutdown.

## Unit Tests

- `delegation` validation rejects absolute paths, traversal, empty segments, and malformed writable-output paths.
- `worker_sandbox` policy resolution denies unapproved networking and produces a confined profile.
- Adapter launch parameters preserve provider restrictions while wrapping read-only processes in the OS sandbox.
- Policy and routing tests prove model-authored fields cannot widen network or filesystem authority.
- Cleanup is idempotent and removes only the sandbox-owned output directory.

## Integration / Functional Tests

- A read-only worker cannot edit an existing workspace file.
- A read-only worker cannot create a new workspace file.
- The same worker can write an allowed artifact beneath `BRIDGE_WORKER_OUTPUT_DIR`.
- A network request fails when network access is not authorized.
- A network-enabled request succeeds only when the current Bridge policy permits it.
- Durable audit records contain the effective sandbox mode and authorities.
- Existing shared, isolated, and full-write launch paths continue to use their prior permissions.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- A macOS platform-gated sandbox smoke test verifies workspace denial, output-directory allowance, and default network denial.

## E2E Tests

- Start a read-only Codex worker and a read-only Claude worker against a temporary repository; each can read the repository and write only to its assigned output directory.
- N/A on non-macOS CI beyond fail-closed unit coverage because the selected isolation primitive is macOS Seatbelt.

## Manual / cURL Tests

- On macOS, run the platform-gated `worker_sandbox` tests and confirm `sandbox-exec` denies tracked writes, untracked writes, and outbound `curl` while allowing output-directory writes.
- Inspect the PR diff to confirm no raw credentials, environment dumps, or model-controlled absolute writable paths enter the sandbox profile.
