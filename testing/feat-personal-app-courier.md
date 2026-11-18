# feat/personal-app-courier — Test Contract

Issue #1026, slice 4b-1. Plan: "Plan: OpenCode courier for personal apps (slice 4b-1)" in Bridge Notes.

## Functional Behavior

- A personal app's connection may be on OpenCode. OpenCode's capability is `gatedCourier` for reads and writes and `partial` for inventory: status only, no tool list or annotations.
- **The courier.** Each read and each approved write runs in a hidden OpenCode session:
  - It runs on the connection's account, with a canonical (symlink-free) working directory.
  - Session rules: deny `*`, allow `execute`, ask `<server>_<tool>` (OpenCode's permission name, with characters outside `[A-Za-z0-9_-]` replaced by `_`).
  - It carries no Bridge tools.
  - It requires the OpenCode 2 API.
- **Prompting.** The model is told to make exactly that call with the canonical JSON arguments.
- **Approval.** On `permission.asked`, Bridge replies `once` only when the permission name matches and the canonical JSON of `metadata.input` equals the canonical approved arguments, and only for the first such prompt. Every other prompt is rejected.
- **Result.** The result is the output of the tool call that carried the approved prompt. A failed call, a rejected prompt, the model ending without making the call, or 150 s passing all end the session and return a plain error, never the agent's text.
- **Drafting.** It may resolve to the chat's own OpenCode connection when OpenCode reports the server connected. The draft's tools are recorded as unverified, and reads are classified by Bridge's read vocabulary alone.
- **Reads.** Reads on OpenCode skip the annotation recheck, since there are none, but still need the read vocabulary and the chat's access mode.
- **Writes.** Writes still need a claimed approval token, as in 4a-2.
- **The card.** It says when an app runs through OpenCode: each call uses a model turn, so it is slower, and its tools are not verified.
- Cursor connections remain unsupported in this slice (4b-2).

## Unit Tests

- Permission name sanitization.
- Courier event loop with fake frames:
  - An exact match is approved, and the tool output is returned.
  - A one-byte difference, a wrong permission, a second prompt, or a prompt after approval is rejected.
  - A tool error or the turn ending without a call fails.
  - The deadline fails.
- Courier session rules and the absence of Bridge tools.
- Router:
  - OpenCode capability.
  - Candidates include the chat's own OpenCode connection.
  - Reads take the courier path, and unverified inventory skips annotations but not the vocabulary.
- Draft: an unverified inventory is accepted for OpenCode only.
- Manifest: `opencode` is a connection harness.
- Card: the OpenCode note is shown.

## E2E Tests

- Live (ignored): real `opencode serve` with the fake MCP server through `OPENCODE_CONFIG_CONTENT`.
  - The courier read returns the fake output.
  - The courier write lands byte-equal in the fake call log.
  - A mismatched call is rejected and never reaches the server.
