# feat/pi-harness Test Contract

## Functional Behavior
- Register Pi as a built-in harness, preserving the open wire HarnessId and all existing harness behavior.
- Resolve `pi` on PATH, require the supported Pi 1.x RPC and security interface, and report missing/incompatible executables, bad handshake/catalog data, malformed records, EOF, command failures, and deadlines clearly.
- Supervise a long-lived `pi --mode rpc` child. Frame strictly on LF (accept CRLF, preserve Unicode separators), correlate responses by unique command ID and command name, continuously consume stdout, separately drain stderr, bound frame sizes/queues, and clean up the process group on failed startup, stop, replacement, and drop.
- Discover configured provider-qualified models using get_available_models. Query thinking levels for each model through set_model/get_available_thinking_levels; never cache raw models, endpoints, headers, credentials, or RPC diagnostic payloads. Reject unavailable models/efforts rather than guessing.
- Launch/resume a persisted Pi session with the selected model and effort. Keep process/session identity consistent, support native resume without replaying old events, and reject native fork. Preserve replacement and shutdown guarantees.
- Normalize text/reasoning blocks with stable per-session/message/content IDs and terminal snapshots, tool calls/results, provider usage, retries, compaction, errors, and interruption. Only agent_settled ends a session run; duplicate completion events do not create duplicate items, usage, or run completions. An accepted/handled prompt cannot leave Bridge waiting forever.
- Interrupt by clearing queued input before abort. Send active input through Pi steering. Native compaction forwards customInstructions and emits Bridge compaction lifecycle events.
- Deny project trust explicitly and disable project/global extension discovery, MCP discovery, skills, templates, themes, and context-file discovery. Only Bridge's bundled permission extension is loaded explicitly. Provide an explicit session directory to avoid Pi's pre-trust project sessionDir lookup.
- Mediate every enabled tool through Bridge's extension at Pi's tool_call boundary. Missing UI, extension exceptions, denied/cancelled UI, or an unproven extension handshake fail closed. Permission replies are correlated to actual pending requests. Completion sessions have no tools.
- Advertise only danger_full_access, not read_only/workspace_write/briefings. Reject unsupported scoped worker and briefing starts before spawning. Tool approvals are authorization, not an OS sandbox.
- RPC cannot attach trusted per-turn context: never fold it into user text or claim delivery. Refuse credential context; explicitly document unavailable hidden memory/context injection. Initial compiled instructions use Pi's system-prompt channel.
- Show Pi availability, discovered models/thinking levels, setup instructions, errors, and limitations in existing Harness settings. No automatic installation, model-weight management, credential copying, or extension marketplace.

## Unit Tests
- pi_adapter framing: split/partial records, CRLF, U+2028/U+2029, malformed/non-object/missing-type data, oversized record, and truncated EOF.
- pi_adapter correlation: interleaved events/responses, mismatched command, duplicate/unknown response IDs, rejection, deadlines, EOF.
- pi_adapter catalog: provider-qualified/local IDs, sanitized projection, duplicate/invalid model identities, effort discovery/validation, default selection.
- pi_adapter normalization: reasoning/text final reconstruction and grouping, tool lifecycle/error, cache-aware usage/cost, retries, compaction, agent_end vs agent_settled, duplicate terminals, per-session isolation, aborted/failed settlement.
- Pi permission extension (Node tests): every tool gated, denied/missing UI/exception fail closed, completion readiness, explicit RPC-only activation.
- security declarations, launch arguments, credential-context refusal, sandbox/briefing rejection, pending permission reply validation.
- Harness conversion/config/defaults, registry/model-profile/compatibility fixtures, and Settings rendering/model controls.

## Integration / Functional Tests
- Fake Pi executable scripts the real stdio transport: startup/readiness handshake, catalog and effort selection, session start/resume, prompt/steer, permission request/reply, fast completion, cancellation, compaction, process failure, and shutdown.
- Verify failed starts and dropped runtimes reap their supervised children. Exercise concurrent commands, stdout pressure, EOF, and stderr separation without real provider credentials.
- Shared built-in adapter compatibility report and protocol artifact parity remain green.

## Smoke Tests
- `bun run build`.
- `bun run test`.
- `bun run check:builtin-adapters`.
- Run an isolated installed Pi 1.x against its real RPC without a model call: prove Bridge extension readiness, configured model projection and orderly shutdown. No live model generation is required when no provider is configured.

## E2E Tests
- Automated fake-Pi user journey: start session, prompt, stream reasoning/text/tool permission/result/usage, settle, interrupt a subsequent run, resume saved session, and shut down without an orphan.
- Live paid-provider generation and packaged desktop UI: manual follow-up when an authenticated Pi provider is available; fake transport and Settings component tests cover this PR's deterministic flow.

## Manual / cURL Tests
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core pi_adapter`.
- `node --test scripts/test/pi-permission.test.mjs`.
- `bunx vitest run src/components/settings/HarnessesPage.test.tsx`.
- Inspect generated compatibility report: Pi uses native RPC, vendor credential ownership, runtime model catalog, and only danger_full_access.
- Settings > Coding agents > Pi: verify availability/reason, configured model labels, reported effort options and scope notice. Sign-in/provider setup is performed in Pi's own CLI/store.
- cURL: N/A; Pi uses supervised stdin/stdout, no inference HTTP API is added.
