# Hash references in chat: test contract

## Functional behavior
- Inline references such as `issue #389` and leading non-agent references such as `#389` or `#eng-alerts` are sent as ordinary chat text.
- A leading configured agent name, id, role, or alias still invokes the specialist; the host remains responsible for resolving and validating it.
- Real shortcut failures retain the draft and show the error. Attachments and selected browser context remain supported for ordinary hash references.

## Unit tests
- Exact agent identity and role aliases match without treating substring matches as shortcuts, including disabled and ambiguous configured targets.
- Inline references, unknown hashtags, and issue numbers are not agent shortcuts.

## Integration and smoke tests
- In the chat composer, send inline and leading hash references unchanged through normal input, without agent dispatch.
- Verify shortcut dispatch and error recovery, and send a non-agent hashtag with browser selection context.

## E2E and manual tests
- In a workspace chat, send `Please inspect issue #389`, then `#389 needs investigation`; neither should raise an agent shortcut error.
- Send `#verifier check this change` and confirm the specialist is dispatched.
