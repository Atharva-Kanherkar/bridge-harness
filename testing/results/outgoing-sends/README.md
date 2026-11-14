# Outgoing send validation

Complete unedited command output, including warnings, failures and skipped tests. Decompress with `gzip -dc <file>`.

| Output | Command | Exit |
|---|---|---|
| [build.log.gz](build.log.gz) | `bun run build` | 0 |
| [test.log.gz](test.log.gz) | `NODE_OPTIONS=--no-experimental-webstorage bun run test` | 0 |
| [handshake.log.gz](handshake.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked handshake` | 0 |
| [initial-node26-failure.log.gz](initial-node26-failure.log.gz) | `bun run test` | 1 |
| [initial-connector-guard-failure.log.gz](initial-connector-guard-failure.log.gz) | `NODE_OPTIONS=--no-experimental-webstorage bun run test` | 101 |

The initial Node 26 failure is its experimental `localStorage` override; disabling it lets jsdom supply browser storage. The initial connector guard failure was fixed by isolating provider-specific logic in the Outgoing module. The final full run passes. The later protocol boundary was verified with 200 protocol unit tests and the filtered workspace handshake run. The final receipt-wrapper refinement then passed all 7 Slack provider tests, including the nested authored-message fixture.

Fresh Chromium profiles verified draft → exact text/image approval → keyboard cancellation → approval → sent permalink, light/dark appearance, focus containment and Copy image after sending. Screenshots are in `testing/screenshots/`.

No messages were posted to real Slack or GitHub recipients for testing. Provider execution uses offline fixtures; a real connected send remains a documented manual check.

## Review regression validation

| Output | Command | Exit |
|---|---|---|
| [review-build.log.gz](review-build.log.gz) | `bun run build` | 0 |
| [review-frontend.log.gz](review-frontend.log.gz) | `NODE_OPTIONS=--no-experimental-webstorage bunx vitest run src/components/OutgoingCard.test.tsx src/components/ConnectorPane.test.tsx src/api.test.ts` | 0 |
| [review-lifecycle.log.gz](review-lifecycle.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core outgoing:: --lib` | 0 (tests) |
| [review-initial-compile-failure.log.gz](review-initial-compile-failure.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core outgoing --lib` (first revision) | 101 |
| [review-providers.log.gz](review-providers.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core outgoing --lib` | 0 |
| [review-connectors.log.gz](review-connectors.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core connector --lib` | 0 |
| [review-github.log.gz](review-github.log.gz) | `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core github_surface --lib` | 0 |
| [review-browser.log.gz](review-browser.log.gz) | Chromium short-card layout, keyboard and send checks at 360/1280px in light/dark | 0 |
| [review-browser-sidebar-failure.log.gz](review-browser-sidebar-failure.log.gz) | First browser runner: attempted to click a hidden narrow-screen sidebar | 1 |
| [review-browser-init-failure.log.gz](review-browser-init-failure.log.gz) | Second browser runner: theme init incorrectly ran in sandboxed frames | 1 |

The first lifecycle test command passed all 48 tests, but its shell wrapper then exited 1 with `zsh:68: read-only variable: status`; subsequent wrappers use `check_exit`. The initial provider compile failed at test call sites for the changed optional prompt signature and a misspelled JSON helper. Those were corrected before the passing provider/connector/GitHub runs. Browser setup was corrected by selecting the chat before resizing and limiting theme initialization to the top frame; all four real layout/send checks then passed with no page errors. The browser runner is retained as [review-browser.cjs](review-browser.cjs); it requires Playwright and the local Vite server on port 1492.

Receipt regressions cover optional audit failure, abandoned versus active executors, editable in-flight fork copies, GitHub host binding, redirects and safe fallback links, bounded CLI timeout, Slack rich-text channel mentions and clearing copied failure alerts.

The final recovery-token refinement then passed all 65 outgoing-filtered tests again: [review-recovery-token.log.gz](review-recovery-token.log.gz), exit 0. This simulates a crash between the durable state claim and token consumption; recovery revokes the surviving token before making the item retryable. The full workspace run began before this final refinement; the focused rerun verifies the final source.

The complete workspace run exited 0: [review-full-test.log.gz](review-full-test.log.gz), command `NODE_OPTIONS=--no-experimental-webstorage CARGO_TARGET_DIR=/Users/atharva/Documents/harness/src-tauri/target bun run test`. It passed 4,392 frontend tests (5 skipped), 3,850 core tests (34 ignored), and all remaining workspace suites, including 135 shell tests and 200 protocol tests.

Linux AppImage packaging was not run locally on macOS. The previous PR CI run passed its tests and Debian bundling, then failed with `failed to run linuxdeploy`; the next branch push reruns that packaging check.
