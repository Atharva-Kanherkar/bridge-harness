# fix/browser-clone-signin: test contract

Locked before implementation using issue 783 and the existing clone contracts.

## Functional Behavior

- Import cookies for the requested domain and every explicitly approved additionalDomains entry, including their subdomains. Validate the complete list before opening the store or fetching its key. Deduplicate overlapping selections.
- Never select, decrypt, or log cookies outside that list. docs.google.com plus accounts.google.com must exclude .google.com, drive.google.com, and docs.google.com.evil.test. A Google parent-domain session requires explicit google.com approval; explain that in the agent instructions.
- Identical cookie values legitimately shared by multiple approved hosts can travel to any of their registered owners, and remain blocked to other hosts.
- Register each imported cookie value with its actual cookie host before loading the clone. A dependency session can travel to its owner and is scrubbed from agent results; it cannot travel to another approved dependency.
- Both the global inbox and dock approval card show the complete cookie domain list and explain that subdomains are included. Blank mode explains that no cookies are copied, while showing allowed connections.
- Pending consent stays immutable. Identical requests reuse their identity; mismatched requests and stale polling return readable errors through the actual HTTP/helper boundary.
- Requesting a replacement leaves the current browser alive until approval. Successful approval reports that the previous browser was destroyed and provides fresh instructions. Failed startup reports a value-free cause and cleans up its clone.

## Unit Tests

- browser_clone_signin: synthetic in-memory SQLite rows verify multi-domain import, explicit parent consent, suffix protection, overlap deduplication, validation, and exclusion of deliberately undecryptable unapproved rows.
- clone_orchestrator: injected synthetic cookie importer verifies the entire approved list reaches import, Storage.setCookies receives all selected cookies once, the guard uses actual host ownership, and import failure tears down the temporary clone.
- browser_clone_guard: shared values are allowed to each registered owner, blocked to an unrelated approved host, and a distinct foreign value remains blocked even when a shared value is also present.
- clone_browser_tool: immutable pending request, stale request status, and replacement acknowledgement cases return meaningful messages.
- CloneRequestInbox.test.tsx: every approved cookie host is visible before Allow, subdomain scope is explained, and Blank mode shows no-copy wording.

## Integration / Functional Tests

- Execute the generated request helper as a child of the bound runtime. A conflicting pending request exits unsuccessfully but prints parseable JSON with the non-empty error; stale status behaves the same way.
- Use the synthetic guarded browser to approve a replacement, prove old clone teardown, and verify the response advertises replacement and refreshed instructions.
- Existing browser-clone guard, lifecycle, takeover, multi-chat, and approval suites remain green.

## Smoke Tests

- bun run build and bun run test must pass before opening the PR.
- cargo check --manifest-path src-tauri/Cargo.toml --workspace verifies native production compilation.

## E2E Tests

- Verify a synthetic two-host imported session in real Chrome: both cookies are stored as session cookies, and the shared session authenticates a local HTTP page through the guard.
- Run available BRIDGE_CLONE_LIVE=1 browser lifecycle tests using isolated blank profiles and synthetic sessions. Never import a real user cookie store without an approved browser capability.
- A signed-in Google Docs journey requires explicit approval for google.com plus any necessary CDN hosts. Record whether it was verified on the patched app; do not equate synthetic tests or the currently running app with that verification.

## Manual / cURL Tests

- Call the generated clone-request helper with docs.google.com and additionalDomains containing accounts.google.com. The approval names both hosts and subdomains, with no implicit parent.
- For a Google session stored on .google.com, explicitly request google.com in additionalDomains. Review that scope on the approval card before allowing it.
- Repeat with a changed additionalDomains list while awaiting approval: JSON explains that the current request must be resolved and no consent or browser is silently replaced.
- After a live clone, approve another request: request_status says the earlier clone was destroyed and returns the new tool instructions.
