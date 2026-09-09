# fix/harness-capability-projection - Test Contract

## Functional Behavior

- Claude chat sessions load user, project, local, and plugin skills while Bridge keeps inherited user hooks and permission policy from changing Bridge's own tool policy.
- Claude read-only workers can discover the same read-only user capabilities as chat sessions: skills, commands, agents, healthy plugins, and configured MCP connectors. Credentials remain referenced from their canonical location and writes remain confined to the worker output directory.
- Claude read-only tool policy has one unambiguous outcome for every tool: permitted tools run and prohibited tools are absent or rejected by an explicit Bridge policy callback. A tool must never be advertised merely because `allowedTools` was mistaken for an availability allowlist.
- Codex read-only workers retain user skills, plugins, global instructions, agent marketplace content, authentication, and MCP configuration while writes remain confined to the worker output directory.
- Codex uses the repository as both process and thread cwd so project `.codex` configuration and skills resolve consistently.
- Claude, Codex, and OpenCode skill installation, discovery, slash expansion, worker projection, and capability reporting use one shared per-harness root definition, including project roots.
- Capability inventory reports only capabilities actually supplied to a session. It distinguishes projected, withheld, unavailable, connected, and authentication-required capabilities where the harness exposes that state.
- Worker capability projection emits a structured `worker.capabilities_projected` event whose payload identifies the harness, projected paths, withheld paths/reasons, and projection failures without leaking credential values.
- Compiled session instructions include a compact, stable capability summary derived from the actual launch configuration, without adding volatile state to the stable prompt prefix.
- Claude plugins or MCP connectors known to be unhealthy or authentication-required are not attached automatically. Explicitly session-enabled capabilities remain representable.
- Child harness processes receive a deterministic PATH that includes Bridge's standard executable search locations while preserving existing PATH entries.
- Codex app-server runs under the same parent-death supervision as other long-lived adapters.
- OpenCode's per-message `system` field behavior is verified against the supported server contract. Bridge uses a compatible prompt mechanism that does not remove OpenCode's built-in skill/tool guidance.
- OpenCode rejects unsupported runtime versions below the feature-compatible floor used by this implementation. Codex rejects unsupported app-server versions or proves required capabilities through protocol probing.
- Capability discovery failures and parser format changes produce diagnostics instead of silently becoming an empty capability set.
- Sandbox documentation matches the implemented HOME and config-directory isolation behavior.

## Unit Tests

- Shared capability-root tests cover all harnesses, user roots, project roots, stable ordering, deduplication, missing paths, files versus directories, and paths containing spaces.
- Projection tests create temporary home/worktree/output trees and assert the exact Claude and Codex links/copies, withheld entries, idempotent reruns, broken-link repair, and collision handling.
- Projection tests assert no secret file contents or credential values are copied into event payloads or logs.
- Claude sidecar option tests assert user settings/skills are enabled, Bridge permission policy remains authoritative, and read-only tool availability does not use `allowedTools` as a restriction.
- Claude MCP/plugin filtering tests cover connected, authentication-required, failed, unknown, timeout, malformed JSON, and changed CLI-output cases.
- Claude context-inventory tests assert it describes the post-filter launch configuration, not the pre-filter discovered configuration.
- Codex launch tests assert supervised spawning, repository process cwd, redirected writable state, preserved capability roots, and hydrated PATH.
- Slash/marketplace tests assert installed and expandable skills use the same shared roots and include project-level roots for Claude, Codex, and OpenCode.
- Capability-summary tests cover chat versus read-only worker, connected versus withheld MCP, denied tools, deterministic ordering, bounded output, and no capabilities.
- OpenCode request tests assert Bridge instructions preserve built-in system behavior on the supported API shape.
- Version-gate/capability-probe tests cover supported, unsupported, malformed, and unavailable runtime versions.

## Integration / Functional Tests

- A fixture user skill for each harness appears in chat and read-only-worker capability discovery.
- A fixture project skill for each harness appears in slash discovery and the matching worker/session inventory.
- A fixture Claude MCP server visible in chat is visible in a read-only worker unless explicitly withheld, with any withholding named in the projection event.
- A fixture Claude authentication-required plugin is excluded from the launch configuration and reported as withheld.
- A read-only Claude fixture can invoke an allowed skill/read capability; a prohibited mutation capability is unavailable or explicitly denied and recorded.
- A Codex read-only fixture resolves global and project skills while attempts to write outside the output directory fail.
- A bare-command fixture MCP executable placed in a standard Bridge search location starts under a minimal GUI-style inherited PATH.
- Killing the supervising Bridge-side process terminates the Codex app-server fixture and its child fixture process.
- Existing briefing, catalog-discovery, adapter resume, sandbox, marketplace, slash, prompt-compiler, and context-inventory suites remain green.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- `bun run check` succeeds.
- Focused Rust tests for bridge-core capability projection, adapters, sandboxing, marketplace, slash handling, prompts, and inventory succeed.
- Claude sidecar Node tests succeed.
- No generated credentials, temporary homes, runtime fixtures, or checkpoint JSON files are tracked by git.

## E2E Tests

- Launch a disposable Claude SDK fixture with isolated user/project skill roots and verify its init inventory and read-only policy behavior without contacting paid external services.
- Launch a disposable Codex app-server fixture through the adapter command builder and verify cwd, environment, supervision, and projected roots without contacting paid external services.
- Launch or schema-test the pinned OpenCode runtime request contract and verify Bridge instructions augment rather than erase built-in skill/tool guidance.
- If a real installed harness is used for an additional smoke test, do not mutate user configuration and do not make paid model calls.

## Manual / cURL Tests

- Inspect `git diff origin/main...HEAD` and confirm every changed production path maps to an issue #579 acceptance item.
- Inspect the structured capability event fixture and confirm it contains paths/statuses only, never file contents, tokens, environment variable values, or credential references.
- Run `gh issue view 579` before final review and account for every root cause and smaller contributing bug in either implementation or an explicit evidence-backed non-bug conclusion.
- No cURL test is required: this change is local harness process/configuration plumbing and must not call external APIs.
