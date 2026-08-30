# feat-cursor-chat-surfaces — Test Contract

## Functional Behavior

- Cursor appears on every provider seam the other three harnesses already occupy: the harness label map, the usage widget's provider list, the usage provider union, the configured-harness list, and the health adapter roster.
- The usage widget offers Cursor its own tile, its own quota reading, and its own sign-in pane, driven by the same descriptor fields as the other providers.
- A named authentication state outranks install status when a tile is labelled. Cursor establishes availability by opening a session, so a signed-out install reports `available: false` and `authState: "signed_out"` together and must render as not signed in with a sign-in button, never as not installed.
- `startProviderLogin("cursor")` runs the vendor's own `login` flow in a Bridge-owned PTY. The executable is resolved by the adapter, which prefers a managed payload and refuses the ambiguous `agent` name: the identity check lives in the probe, which a login never reaches, so a foreign executable sharing that name is never spawned behind a pane labelled Cursor.
- Cursor is a built-in managed agent with a runtimes card of its own. Its recipe is a per-platform release artifact pinned by digest, since the vendor publishes no npm package; there is no Windows build, and installing on Windows reports the platform as unsupported rather than fetching a url that does not exist.
- A release-artifact receipt carries the vendor's own version string rather than a digest prefix, and its recorded source carries the digest, so a corrected pin under an unchanged version reinstalls instead of short-circuiting as already current.
- A release version is validated as a safe path component before any fetch, because staging interpolates it into a directory name before the payload engine's own check runs.
- `cursor_adapter::locate` prefers a Bridge-managed payload over PATH, the way every other adapter resolves its runtime; an installed payload is launchable, and the harness stops reporting itself as absent once one is installed.
- The runtimes card and the adapter agree about whether Cursor is installed. Both accept `cursor-agent` and the ambiguous `agent`, so a working install is never described as absent and never offered a second copy, and the uninstall guard that refuses to remove an unmanaged runtime sees the same file.
- An ACP turn persists the assistant reply it streamed. The protocol emits deltas and then ends the prompt, and the forest stores deltas as transient, so the session assembles one `message.completed` per message.
- Assembly ends a message where ACP ends one — a chunk carrying a different `messageId` starts a new message — and flushes the prose preceding a tool call before that call, so the transcript keeps its order.
- The assembled message carries the turn's own ending as its status. A cancelled, refused, or token-truncated reply is not persisted as a finished one, because context projection replays that text to the model and delegation parsing runs over it.
- A reply is persisted when the turn fails as well as when it succeeds: an agent error or a closed transport publishes what streamed, marked interrupted.
- An assembled message with no `messageId` of its own is given a unique identity, because the delegation ledger keys its dedupe receipt on that id and a shared key discards every directive after the first.
- The accumulator is armed at turn start, so deltas arriving with no prompt outstanding cannot open the next turn's message or latch its id, and it is bounded like every other sink in the module.
- Replayed history is assembled the same way as a live turn. A replayed identity is preserved rather than synthesized, so a second reload fingerprints identically and adds nothing.

## Unit Tests

- Rust `cursor_adapter` tests cover the resolution order (managed payload, published name, ambiguous name), the sign-in refusal of the ambiguous name, and the distinct remedy sentence each unavailable state produces.
- Rust `managed_runtime` tests cover the release-artifact version validation, the derived recipe count on a platform where the vendor ships no Cursor build, and the exact-pinning assertions each recipe already carries.
- Rust `acp_session` tests cover two messages in one turn, the tool-call boundary and its effect on event order, the per-turn identity of an unnamed message, persistence on the error path, the status carried for each stop reason, the turn-start barrier, and the accumulation cap including a multi-byte cut.
- Frontend `UsageWidget` tests cover the signed-out Cursor descriptor in the shape the daemon actually reports and assert on the sign-in button rather than on prose the page emits regardless.
- Frontend `ManagedAgentsPanel` tests cover the Cursor card alongside the other three.

## Integration / Functional Tests

- A reload assembles replayed deltas into messages, preserves their order and identity, and a second reload of the same history admits nothing.
- The managed-agents list reports Cursor with a backing and state derived from the same resolution the adapter uses.
- Frontend and Rust suites remain green together.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- `bun run check` succeeds.

## E2E Tests

- Covered by scripted-agent fixtures rather than a live vendor process: driving the real Cursor CLI would consume vendor capacity and require a signed-in account. The scripted agent proves the assembly, ordering, status, and replay contracts end to end over the real transport.

## Manual / cURL Tests

- With no `cursor-agent` on PATH, install Cursor from the runtimes card and confirm the harness becomes available without a restart and a session starts against the managed payload.
- With a signed-out Cursor, open the usage widget and confirm the tile reads not signed in with a working Sign in button rather than not installed.
- With Cursor on PATH only as `agent`, confirm the runtimes card reports it as external rather than offering an install, and that Sign in refuses with the sentence naming `cursor-agent`.
- Run a Cursor turn, reload the app, and confirm the reply is still there; interrupt a turn mid-sentence and confirm the partial reply persists and is not labelled completed.
- No cURL test is applicable: these are Tauri invoke and protocol flows rather than HTTP endpoints.
