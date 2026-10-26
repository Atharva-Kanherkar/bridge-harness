# fix/local-browser-routing - Test Contract

## Functional Behavior
- Local app QA (localhost, 127.0.0.1, IPv6 loopback), screenshots, and extension testing use available Playwright/browser automation without requesting a cookie clone.
- Public pages that need no existing login use ordinary browsing or fresh browser automation.
- If automation is unavailable, report or resolve that limitation; do not automatically request a clone.
- Request a Bridge clone when the task needs the user's existing signed-in browser state, or when the user explicitly requests a Bridge clone or cookie import. Local apps can also explicitly need that state.
- Preserve explicit domain consent, parent-domain cookie rules, polling, replacement, extension loading, and cleanup instructions. Do not block legitimate localhost clone requests or change browser authorization enforcement.

## Unit Tests
- Generated request context communicates the browser selection boundary and retains the capability protocol.
- Existing generated-helper, consent, stale-request, and cleanup tests pass unchanged.

## Integration / Functional Tests
- Live-turn context injection still delivers the revised instructions through the trusted context channel.
- Existing clone request/approval/handoff/cleanup integration test passes.

## Smoke Tests
- `bun run build` and `bun run test` pass in the isolated worktree.

## E2E Tests
- Run opt-in live Codex routing evals through Bridge's production Codex adapter with the actual generated per-turn clone context, independent sessions, and a bounded deadline.
- Cases cover local IPv4/IPv6/localhost QA, public pages, extension testing, unavailable automation, signed-in tasks, explicit clone requests, and local apps requiring existing login state.
- Record individual results and compare the old and revised prompt on the same cases. Provider errors and tool attempts fail the eval rather than scoring as routing decisions.
- This is a browser-selection eval; full website interaction and other providers are outside this change's E2E scope.

## Manual / cURL Tests
- Inspect the generated context in the live eval output: ordinary testing must not mandate clone approval, and signed-in state must still require explicit consent.
- Run `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core live_browser_selection_eval -- --ignored --nocapture` with authenticated Codex installed.
- No external-site cURL or cookie access is needed for this prompt-only change.
