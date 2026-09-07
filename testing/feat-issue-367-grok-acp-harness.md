# feat/issue-367-grok-acp-harness — Test Contract

Issue #367: Grok Build Native ACP Harness (Phase 1 & Phase 2).
Reuses the shared ACP client layer (`acp_session.rs`, `acp_events.rs`, `acp_registry.rs`) to integrate the official xAI Grok Build CLI (`grok`) as a native stdio ACP harness in Bridge.

Locked before implementation.

## The shape of the problem

Bridge integrates vendor coding agents (Claude, Codex, Cursor, OpenCode). Grok Build ships an Agent Client Protocol (ACP) server directly within its native binary (`grok agent --no-leader stdio`), requiring no Node sidecar (unlike Claude) and no custom HTTP server (unlike OpenCode).

Two load-bearing invariants govern this integration:
1. **Strict 1:1 Subprocess Supervision (`--no-leader`)**: Bridge must own the process lifecycle (PID tracking, OS signal propagation, stdout/stdin pipe lifecycle, worktree containment) and deliberately avoids Grok's background daemon/leader mode to prevent worktree lock contention and leaked background daemons.
2. **Vendor Auth Hygiene & Redaction**: Bridge never stores, copies, or persists Grok credentials or cookies. Configured API keys (`XAI_API_KEY` or `GROK_API_KEY`) are passed strictly via child process environment variables and are redacted from all diagnostic logs, failure context, and events.

## Functional Behavior

### Discovery and Locating
- Bridge looks for the vendor's executable `grok` using `locate_with(...)`:
  1. `BRIDGE_GROK_BIN` environment variable override.
  2. Bridge-managed entrypoint if present.
  3. User-configured custom binary path.
  4. System `PATH` resolution via `binary::resolve("grok")`.
- When the binary is absent, the harness reports `GrokUnavailable::NotInstalled` with actionable advice.
- When the binary version cannot be read, reports `GrokUnavailable::UnreadableVersion`.

### Protocol Confirmation & Probing
- Before offering the harness, Bridge completes an ACP initialize handshake under a 12s timeout (`PROBE_TIMEOUT`).
- Non-protocol stdout or ANSI terminal output is treated as a failed probe rather than noise.
- Handshake failures, auth requirements, or timeouts classify cleanly into `GrokUnavailable` variants:
  - `NeedsSignIn`: Vendor requires authentication; directs user to `grok login`.
  - `NotProtocol`: Output was terminal text/ANSI rather than JSON-RPC ACP.
  - `ProbeFailed`: Process crashed or exited with error.
- Probe results are cached against `(executable_path, version)` to avoid re-probing on hot status reads.

### Launch and Process Ownership
- Session launch constructs `AcpLaunch` with arguments `["agent", "--no-leader", "stdio"]`.
- Child process runs as a process group leader, supervised and terminated via `terminate_process_group` on drop/shutdown.
- CWD of the child process is set to the isolated session worktree directory.
- `KEY_VARIABLES` (`["XAI_API_KEY", "GROK_API_KEY"]`) are injected into the child process environment if configured.

### Capabilities and Dynamic Catalog
- Models and modes are read dynamically from the negotiated ACP session configuration (`session/new` response).
- Default model selects the advertised standard tier model or first available model.
- Native resume (`session/resume` or `session/load`) is attempted only if the negotiated Grok capabilities advertise support; otherwise, Bridge gracefully falls back to session forest checkpoint handoff.

### Security and Redaction
- All stderr tails, diagnostic messages, and probe errors are passed through `redact(...)` to strip any active API keys.
- Bridge `PolicyCoordinator` remains authoritative over write permissions and tool executions.

## Unit Tests

Specific unit tests in `src-tauri/bridge-core/src/grok_adapter.rs` and `adapters.rs`:
- `test_locate_finds_grok_on_path`: Verifies resolution order (managed -> override -> PATH).
- `test_locate_reports_not_installed`: Verifies clean `NotInstalled` error when missing.
- `test_locate_reports_unreadable_version`: Verifies error when version execution fails.
- `test_launch_for_constructs_no_leader_args`: Verifies `["agent", "--no-leader", "stdio"]` arguments.
- `test_launch_for_injects_env_keys`: Verifies API key injection into process environment.
- `test_redact_sanitizes_keys`: Verifies key redaction from logs and error strings.
- `test_probe_cache_invalidation`: Verifies cache hit on matching (path, version) and invalidation on change.
- `test_classify_probe_failure_modes`: Verifies mapping of `AcpError` to `GrokUnavailable` variants.
- `test_adapter_descriptor_matches_contract`: Verifies ID `"grok"`, label `"Grok Build"`, and standard tier.
- `test_grok_context_inventory`: Verifies catalog and per-turn context inventory segments.

## Integration / Functional Tests

Deterministic offline fake-ACP CLI tests via `testing/fixtures/grok-fake-cli.sh`:
- Fake CLI verifies `--no-leader` argument is present.
- Fake CLI simulates valid ACP v1 initialize + session/new handshake.
- Fake CLI simulates authentication requirement (error code 401 / auth required).
- Fake CLI simulates streaming turn updates, assistant text tokens, and tool calls.
- Fake CLI simulates session shutdown and process group reaping.
- Rust integration tests verifying `GrokAdapter::start`, event forwarding, and teardown against the fake CLI.

## Smoke Tests

- `AdapterRegistry::built_in()` contains the `grok` harness.
- Registry descriptors report `grok` alongside `claude`, `codex`, `cursor`, and `opencode`.
- `cargo check --manifest-path src-tauri/Cargo.toml --workspace` succeeds without warnings.

## E2E Tests

- N/A — Live Grok tests are opt-in (`#[ignore]`) and only run when a live `grok` binary is present on PATH.

## Manual / cURL Tests

Verification commands:
```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core grok_adapter
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core adapters::
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core builtin_compatibility
cargo check --manifest-path src-tauri/Cargo.toml --workspace
```
