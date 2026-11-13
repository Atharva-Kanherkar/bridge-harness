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
