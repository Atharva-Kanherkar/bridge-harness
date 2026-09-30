# Clone stack landing review

## Locked expectations

- All required build and test commands must pass without ignoring failures.
- Switching chats must immediately scope clone reads and actions to the new chat.
- Takeover input tests must exercise a real asynchronous API contract and report errors.
- Destroying a session must revoke pending requests even when no browser was created.
- Tool listeners and browser processes must stop when their owners are dropped.
- Failed startup must destroy its newly created browser before returning an error.
- Approval must refer to the request the person actually saw and the requesting runtime.
- Approval must make the browser available to the requesting agent without another user message.
- User settings, sign-in inside the clone, extension loading, and cleanup must be reachable in the production app.
- Sensitive page content must not be falsely described as redacted.

## Verification

Run `bun run build`, `bun run test`, the clone unit tests, and protocol checks.
Live tests must use synthetic local fixtures without reading real browser profiles or Keychain.
Review GitHub checks on the exact final commits before merging.

## End-to-end acceptance

From the packaged application, ask an agent to use a browser; see the approval;
allow it; observe the agent receive and use the browser; end the task and verify
cleanup. Repeat in the same chat and in concurrent chats. A direct orchestrator
test is supporting evidence, not a substitute for this application path.

## Review disposition

Track confirmed blockers and their resolution in the final review report. Do not
merge while any consent, isolation, confidentiality, or lifecycle blocker remains.
