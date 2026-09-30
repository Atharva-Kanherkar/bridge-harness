# Chat search measurements — 2026-09-30

The index benchmark passes its latency target. The completed Claude tool-free run
meets the token target after removing the coding preset and tool schemas,
but misses overall recall and elapsed-time targets. The final warm-session
implementation has not yet had a successful Claude live rerun: Claude returned
a 429 session limit, resetting at 15:30 Asia/Kolkata. Codex CLI now independently
evaluates the shared funnel; its measured results are below.

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

## Codex CLI evaluation

Codex CLI 0.157.1, `gpt-6-luna`, low effort, 200 synthetic chats / 20,000
entries and the same 40 labelled queries. Both runs made 18 deep searches.
The evaluator ignores user configuration, disables native integrations,
uses read-only sandboxing, rejects native tool events, and writes usage under
`provider.codex`. It replays the search's text history through a fresh
`codex exec` on each turn. These token and timing figures include that CLI's
remaining harness overhead and per-turn startup; they are separate from the
production Claude transport.

| Metric | Production 8 s budget | Diagnostic 30 s budget |
|---|---:|---:|
| Exact recall@4 | 15/15 | 15/15 |
| Vague recall@4 | 6/15 | 10/15 |
| Time + topic recall@4 | 10/10 | 10/10 |
| Overall recall@4 | 31/40 (77.5%) | 35/40 (87.5%) |
| T2 median reported tokens | 10,933 | 43,946 |
| T2 maximum reported tokens | 21,917 | 77,783 |
| T2 median elapsed time | 8,029 ms | 18,526 ms |
| T2 maximum elapsed time | 8,239 ms | 27,116 ms |
| T2 fallbacks | 14/18 | 3/18 |
| Tagged usage rows | 14 | 44 |
| Unsettled search sessions | 0 | 0 |
| Forest entries written by search | 0 | 0 |

Token figures cover completed turns with provider-reported usage; interrupted
turns may not report final usage. Codex's input includes cache reads, which
the evaluator does not add again. The 30-second diagnostic improves vague
recall but still misses the 90% overall target. It does not demonstrate the
eight-second production budget or three-second latency target. Production
search continues to use the certified Claude briefing adapter.

```sh
BRIDGE_CHAT_SEARCH_LIVE_HARNESS=codex \
  CARGO_TARGET_DIR=src-tauri/target-wt cargo test --manifest-path src-tauri/Cargo.toml \
  -p bridge-core --release --lib chat_search::eval::live_fixture -- --exact --ignored --nocapture
```

Add `BRIDGE_CHAT_SEARCH_LIVE_WALL_SECONDS=30` for the diagnostic run.
`BRIDGE_CHAT_SEARCH_LIVE_MODEL` selects another available Codex model.
The wall override is confined to the ignored evaluator; production limits
remain unchanged.

## Real database smoke probe on Codex

A consistent SQLite read snapshot was backed up to a separate temporary
directory: 3,869,351,936 bytes in 16.8 seconds. An earlier backup without a
held read snapshot repeatedly restarted as the app wrote and was cancelled
after 180 seconds; its partial copy was removed. Opening and migrating the
successful copy took 28.6 seconds. The live app database was only opened
read-only for backup; the probe ran on the copy without booting/reaping any
of the live app's recorded adapter processes.

Codex CLI with the labelled 30-second diagnostic budget:

| Query | T1 time | Deep result | Deep elapsed | Reported tokens | Lookups |
|---|---:|---|---:|---:|---:|
| plugins catalog stall | 43 ms | index fallback (budget) | 30,087 ms | 57,792 | 3 |
| finding an old conversation from a vague memory | 62 ms | model answer | 27,061 ms | 70,217 | 3 |
| the work about searching chats | 2 ms | model answer | 22,358 ms | 67,002 | 2 |

The probe passed, recorded 10 usage rows across three hidden sessions, left
zero unsettled search sessions, and wrote zero search forest entries. These
queries have no labelled expected answers; this is integration/smoke evidence,
not a recall measurement or a production latency pass.
The temporary database copy and its migration backup were removed after checking
the session, usage, and forest counts.

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

The production-Claude real-database probe remains pending. The same probe
can now select either harness. Use a consistent SQLite backup in a separate
directory, then set `BRIDGE_CHAT_SEARCH_LIVE_DB` to that copy and run
`chat_search::eval::live` with `--exact --ignored --nocapture`.
Set `BRIDGE_CHAT_SEARCH_LIVE_HARNESS=codex` to select Codex CLI.
Queries can be supplied with `BRIDGE_CHAT_SEARCH_LIVE_QUERIES`, separated by
`|`. Never point the measurement at the app's live file: opening the store
applies migrations and search writes its hidden session/usage rows.

## Offline validation

- `bun run build` passed.
- The sidecar, release-script and native menu checks passed.
- All 229 frontend test files / 2,856 tests passed with
  `NODE_OPTIONS=--no-experimental-webstorage` (Node 26 otherwise masks jsdom's
  `window.localStorage`, causing the unchanged updater tests to fail).
- The latest search implementation passed 70 Rust tests, with four live/bench
  tests ignored. Added Codex tests cover cache accounting, native tool rejection,
  and reaping a timed-out CLI process while settling its hidden session.
  The adapter boundary passed all three tests, including the
  discovery-skipping regression test.
- The remaining Rust workspace tests passed on the final implementation:
  `cargo test --manifest-path src-tauri/Cargo.toml --workspace --exclude bridge-core -- --test-threads=8`.
- The latest full `bun run test` passed, including 2,759 core tests (15 ignored),
  all frontend tests, sidecar/native/release checks, the remaining Rust workspace
  tests, and doc tests. Exact command:
  `NODE_OPTIONS=--no-experimental-webstorage RUST_TEST_THREADS=8 CARGO_TARGET_DIR=src-tauri/target-wt bun run test`.
  The bounded Rust test concurrency also passed the unchanged
  `provider_usage::claude_cli::tests::startup_prompt_aborts_without_writing_a_command`,
  which failed in an earlier full run and passed on its immediate isolated retry.
  No test was skipped or changed to obtain the clean full-suite result.
