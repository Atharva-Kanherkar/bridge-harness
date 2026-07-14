# Issue 42 Contract: Chat secret interception only

## Scope

- Before a user-authored chat turn reaches any structured harness adapter, Bridge classifies the outbound text locally for supported credential patterns.
- Detected credential values are replaced with per-message opaque references. The harness receives the surrounding user text and references, never the detected values.
- The sanitized user turn is the only form eligible for durable session history, UI conversation events, checkpoints, restoration context, handoff projection, logs, or history snapshots.
- Interception is provider-neutral and applies to every harness routed through the shared `send_turn` command.
- This slice does not store, resolve, inject, authenticate, revoke, or use credentials. It adds no OAuth flow, cloud integration, service adapter, Keychain integration, or model-facing tool.

## Functional expectations

1. Recognize high-confidence GitHub, Slack, Notion, OpenAI/Anthropic-style, AWS access-key, bearer/JWT, private-key, and credential-assignment forms without sending the matched value to a harness.
2. Preserve ordinary surrounding prose while replacing each distinct detected value with an opaque reference shaped like `[secret:sec_<opaque>]`.
3. Repeated occurrences of the same value within one message use the same reference; different values use different references.
4. A message without a detected secret is byte-for-byte unchanged.
5. Common source code, UUIDs, commit hashes, URLs, and short configuration values are not classified solely because they contain mixed characters.
6. Claude's locally persisted user event contains the sanitized text. Provider-echoed Codex turns can only echo the sanitized outbound text.
7. Interception metadata may record detector kinds and reference IDs, but never matched values or value-derived hashes.
8. Internal Bridge prompts and worker scheduling remain unchanged; this slice protects user-authored turns entering through the shared chat command.

## Automated verification

- Unit tests cover every supported high-confidence detector, multiple and repeated values, multiline private keys, unchanged safe text, and representative false-positive cases.
- A send-turn boundary test uses a recording adapter and asserts that only sanitized text reaches the harness.
- A persistence test asserts that Claude's durable session entry contains the sanitized form and that the canary secret is absent from serialized session history.
- Run `cargo test --manifest-path src-tauri/Cargo.toml`.
- Run `bun run build` and `bun run test` as required by `AGENTS.md`.
- Run `git diff --check`.

## Manual verification

1. Paste a test token into a direct Codex chat and confirm the visible/sent turn uses an opaque reference.
2. Repeat with Claude and confirm the visible turn uses an opaque reference.
3. Confirm an ordinary code-heavy prompt is sent unchanged.

## Explicit non-goals

- Credential use by Codex, Claude, subagents, shell commands, MCP servers, plugins, or service APIs.
- OAuth or browser login.
- GitHub, Slack, Notion, harness, or cloud credential adapters.
- Keychain storage, process-scoped injection, credential replacement, or revocation.
- General redaction of arbitrary secrets returned by external processes that Bridge never intercepted.
