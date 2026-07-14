# Issue 42 Contract: Chat secret interception and brokered OpenAI use

## Scope

- Before a user-authored chat turn reaches any structured harness adapter, Bridge classifies the outbound text locally for supported credential patterns.
- Detected credential values are replaced with per-message opaque references. The harness receives the surrounding user text and references, never the detected values.
- OpenAI-style `sk-` credentials are retained only in Bridge process memory and bound to the session that submitted them. Other detected credential families remain redaction-only in this slice.
- Wrapped Codex and Claude harnesses receive credential-proxy usage instructions and can call OpenAI through a loopback-only Bridge endpoint using the opaque reference. Bridge attaches the real key only inside the proxy request.
- The sanitized user turn is the only form eligible for durable session history, UI conversation events, checkpoints, restoration context, handoff projection, logs, or history snapshots.
- Interception is provider-neutral and applies to every harness routed through the shared `send_turn` command.
- This slice adds no OAuth flow, cloud persistence, Keychain integration, provider login, or standalone agent implementation.

## Functional expectations

1. Recognize high-confidence GitHub, Slack, Notion, OpenAI/Anthropic-style, AWS access-key, bearer/JWT, private-key, and credential-assignment forms without sending the matched value to a harness.
2. Preserve ordinary surrounding prose while replacing each distinct detected value with an opaque reference shaped like `[secret:sec_<opaque>]`.
3. Repeated occurrences of the same value within one message use the same reference; different values use different references.
4. A message without a detected secret is byte-for-byte unchanged.
5. Common source code, UUIDs, commit hashes, URLs, and short configuration values are not classified solely because they contain mixed characters.
6. Claude's locally persisted user event contains the sanitized text. Provider-echoed Codex turns can only echo the sanitized outbound text.
7. Interception metadata may record detector kinds and reference IDs, but never matched values or value-derived hashes.
8. Worker scheduling remains unchanged; the only internal-prompt change is the provider-neutral credential-proxy calling convention appended to wrapped harness instructions.
9. An OpenAI reference authorizes only requests to a fixed Bridge-configured OpenAI origin and `/v1/` paths. The harness cannot choose another upstream host or supply an authorization header.
10. The loopback proxy supports bounded, non-streaming `GET`, `POST`, and `DELETE` requests, forwards only allowlisted request headers, and returns response status/body without credential-bearing headers.
11. Broker entries are removed when the chat is cleared and when Bridge exits. Secret buffers are zeroized when replaced or removed.
12. Direct chats, orchestrators, and wrapped workers learn the stable proxy calling convention through provider-neutral harness instructions; no custom agent runtime or model tool implementation is introduced.

## Automated verification

- Unit tests cover every supported high-confidence detector, multiple and repeated values, multiline private keys, unchanged safe text, and representative false-positive cases.
- A send-turn boundary test uses a recording adapter and asserts that only sanitized text reaches the harness.
- A persistence test asserts that Claude's durable session entry contains the sanitized form and that the canary secret is absent from serialized session history.
- A fake-upstream integration test asserts that the harness-visible request contains only the opaque reference while the upstream receives the canary in `Authorization: Bearer ...`.
- Proxy tests reject an unknown reference, a reference owned by another session, a non-`/v1/` path, a caller-supplied authorization header, an unsupported method, and an upstream supplied by the caller.
- A cleanup test proves clear/replacement removes and zeroizes the in-memory credential entry.
- Run `cargo test --manifest-path src-tauri/Cargo.toml`.
- Run `bun run build` and `bun run test` as required by `AGENTS.md`.
- Run `git diff --check`.

## Manual verification

1. Paste a test token into a direct Codex chat and confirm the visible/sent turn uses an opaque reference.
2. Ask Codex to make a non-streaming OpenAI request through the documented local proxy and confirm it succeeds without the key appearing in the command or transcript.
3. Repeat with Claude and confirm the visible turn and proxy call use only the opaque reference.
4. Confirm an ordinary code-heavy prompt is sent unchanged.

## Explicit non-goals

- OAuth or browser login.
- GitHub, Slack, Notion, harness, or cloud credential adapters.
- Keychain or SQLite credential storage, environment injection, command-line injection, or persistence across Bridge restarts.
- Arbitrary upstream proxying, streaming API responses, or brokered use of non-OpenAI credential families.
- General redaction of arbitrary secrets returned by external processes that Bridge never intercepted.
