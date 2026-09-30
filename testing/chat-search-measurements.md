# Chat search measurements — 2026-09-30

The index benchmark passes its latency target. The last completed live run
meets the token target after removing the coding preset and tool schemas,
but misses overall recall and elapsed-time targets. The final warm-session
implementation has not yet had a successful live rerun: Claude returned a
429 session limit, resetting at 15:30 Asia/Kolkata.

## Index benchmark

Release build, synthetic corpus expanded to 1,000 chats / 100,000 entries,
40 labelled queries. No provider calls.

| Metric | Measured | Target |
|---|---:|---:|
| T1 p50 | 0.7 ms | — |
| T1 p95 | 1.7 ms | < 50 ms |
| Exact recall@4 | 15/15 | ≥ 90% |
| Vague recall@4 | 6/15 | measured baseline |
| Time + topic recall@4 | 10/10 | ≥ 90% |
| Overall T1 recall@4 | 31/40 (77.5%) | full funnel ≥ 90% |
| Confident without T2 | 22/40 (55%) | expected > 70% |
| Confident but wrong top hit | 0 | 0 |
| T2 first prompt, estimated median / max | 345 / 470 tokens | — |
| Digest text size | 48,784 bytes | — |

Baselines use the same 1,000-chat corpus. Prompt tokens below are character
estimates (`chars / 4`); these are prompt-size comparisons, not live model
ranking measurements. No model inference latency was measured for those
baselines.

| Baseline | Measured |
|---|---:|
| All titles in a prompt | 5,635 estimated tokens |
| Sequential per-session recall across every chat | 141.5 ms/query |
| All digests in a prompt | 12,446 estimated tokens |
| All entries in a prompt | 603,939 estimated tokens |

Command, from the worktree root:

```sh
CARGO_TARGET_DIR=src-tauri/target-wt cargo test --manifest-path src-tauri/Cargo.toml \
  -p bridge-core --release --lib chat_search::eval::bench -- --ignored --nocapture
```

## Completed live measurements before warm sessions

These saved runs use the 200-chat / 20,000-entry fixture and the real Claude
adapter with Haiku. Both evaluated all 40 labelled queries; 18 needed T2.
Provider-reported tokens include input, output, cache reads and cache writes.
Elapsed time includes provider startup and shutdown.

| Metric | Coding preset | Tool-free preset | Target |
|---|---:|---:|---:|
| Overall funnel recall@4 | 32/40 (80%) | 31/40 (77.5%) | ≥ 90% |
| Vague funnel recall@4 | 7/15 | 6/15 | measured |
| T2 median tokens | 18,723 | 1,377 | ≤ 2,500 |
| T2 maximum tokens | 55,226 | 4,839 | — |
| T2 median elapsed time | 18,733 ms | 18,993 ms | ≤ 3,000 ms |
| T2 maximum elapsed time | 27,201 ms | 29,692 ms | — |
| T2 fallbacks | 10/18 | 13/18 | — |
| Tagged usage rows | 23 | 21 | — |
| Unsettled search sessions | 0 | 0 | 0 |
| Forest entries written by search | 0 | 0 | 0 |

Startup dominated these runs. Tool-free launches now bypass plugin and MCP
discovery, whose results an empty scope cannot use. The warm pool starts one session while the
user types, consumes it for one search, rejects expired sessions, and stops
unused sessions after 180 seconds. Unit tests verify its lifecycle; a new
live measurement is required to quantify both changes. The attempted rerun
during this continuation returned only quota errors and was stopped; it is
not performance or recall evidence.

## Remaining evidence

After Claude's quota resets, rerun the final implementation:

```sh
CARGO_TARGET_DIR=src-tauri/target-wt cargo test --manifest-path src-tauri/Cargo.toml \
  -p bridge-core --release --lib chat_search::eval::live_fixture -- --ignored --nocapture
```

This simulates a shallow search, three seconds of typing-to-Enter time, then
the deep request. `BRIDGE_CHAT_SEARCH_THINK_SECONDS` changes that delay.
The measurement aborts on a provider quota failure and checks that search
sessions settle and no forest entries are written. Overall recall and live
elapsed-time targets remain open; do not describe the feature as meeting
them until measured.

The real-database smoke run also remains pending. Use a consistent SQLite
backup in a separate directory, then set `BRIDGE_CHAT_SEARCH_LIVE_DB` to that
copy and run `chat_search::eval::live` with `--exact --ignored --nocapture`.
Queries can be supplied with `BRIDGE_CHAT_SEARCH_LIVE_QUERIES`, separated by
`|`. Never point the measurement at the app's live file: opening the store
applies migrations and search writes its hidden session/usage rows.

## Offline validation

- `bun run build` passed.
- The sidecar, release-script and native menu checks passed.
- All 229 frontend test files / 2,856 tests passed with
  `NODE_OPTIONS=--no-experimental-webstorage` (Node 26 otherwise masks jsdom's
  `window.localStorage`, causing the unchanged updater tests to fail).
- The latest search implementation passed 67 Rust tests, with four live/bench
  tests ignored. The adapter boundary passed all three tests, including the
  discovery-skipping regression test.
- The remaining Rust workspace tests passed on the final implementation:
  `cargo test --manifest-path src-tauri/Cargo.toml --workspace --exclude bridge-core -- --test-threads=8`.
- The full `bun run test` run reached 2,754 passing core tests, 15 ignored, and
  one failure in the unchanged
  `provider_usage::claude_cli::tests::startup_prompt_aborts_without_writing_a_command`.
  That test passed immediately when rerun alone. The full command is therefore
  not yet a clean pass; preserve this qualification when reporting validation.
