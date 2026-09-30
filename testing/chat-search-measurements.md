# Chat search measurements — 2026-09-30

Two targets are met, one is met only at a diagnostic budget, and one is not
reachable on this transport. The numbers below are from the production Claude
adapter unless a row says Codex.

| Measurement | Result | Target | Status |
|---|---:|---:|---|
| T1 p95, 100,000 entries | 0.8 ms | < 50 ms | met |
| T2 median reported tokens, 8 s budget | 1,544 | ≤ 2,500 | met |
| Funnel recall@4, 8 s budget | 32/40 (80%) | ≥ 90% | **not met** |
| Funnel recall@4, 30 s diagnostic | 36/40 (90%) | ≥ 90% | met at 30 s |
| T2 median elapsed, 8 s budget | 8,849 ms | ≤ 3,000 ms | **not reachable** |
| Index-confident queries | 22/40 (55%) | expected > 70% | **ceiling 62.5%** |

`confident but top hit wrong` is 0 in every run: Enter never opens a chat the
index is unsure about.

## Why recall and latency cannot both be met

The deep search needs two model turns on a vague query — one to propose
words the user really typed, one to answer with what that found. Measured on
the production adapter:

| | Measured |
|---|---:|
| Session start (sidecar spawn) | 7–22 ms |
| First model turn | 3,826–7,477 ms |
| Second model turn | 2,263–5,209 ms |

So one turn costs about 4–5 s. **The ≤ 3,000 ms target sits roughly 2.5× below
the cost of a single model turn**, before any lookup has run. It is not
reachable on this transport, and no amount of index work changes that: at an
8 s budget turn 1 alone consumes most of the wall clock, turn 2 is cut off,
and the loop returns the index's own cards — 12 of 18 deep searches still fall
back. Give the loop 30 s and the same code reaches 36/40.

Prefill is not the cost. Trimming the first turn from eight cards to four
moved the median first-turn input by five tokens (most unsure queries return
one to three candidates) while the turn stayed at 3.8–7.5 s. It is
generation-bound, so the cards were put back.

## The recall ceiling is a property of the corpus

Eight of the fifteen vague queries share **no word at all** with the chat they
should find — `bill`→Invoice PDF, `money`→Budget spreadsheet, `toggles`→Feature
flag, `bot`→Chess engine, `messages`→Email bounce. Every background chat in the
fixture is a deliberate lexical decoy of the form `Refactor the <module>`, so
the query's own words are always present in the wrong chats.

T1's lexical ceiling is therefore 32/40, and the model stage has to supply at
least 4 of the remaining 8 for the funnel to reach 90%. At 30 s it supplies 5
and the funnel hits 36/40. The full argument, including a Porter stemmer that
was implemented, measured and rejected, is in
[`chat-search-vocabulary-gap.md`](chat-search-vocabulary-gap.md).

## Index benchmark

Release build, synthetic corpus expanded to 1,000 chats / 100,000 entries,
40 labelled queries. No provider calls.

| Metric | Measured | Target |
|---|---:|---:|
| T1 p50 | 0.3 ms | — |
| T1 p95 | 0.8 ms | < 50 ms |
| Exact recall@4 | 15/15 | ≥ 90% |
| Vague recall@4 | 7/15 | measured, ceiling-bound |
| Time + topic recall@4 | 10/10 | ≥ 90% |
| Overall T1 recall@4 | 32/40 (80%) | — |
| Confident without T2 | 22/40 (55%) | ceiling 25/40 |
| Confident but wrong top hit | 0 | 0 |
| T2 first prompt, estimated median / max | 475 / 499 tokens | — |
| Digest text size | 48,784 bytes over 1,000 chats | — |

Baselines use the same 1,000-chat corpus. Prompt tokens are character
estimates (`chars / 4`); these are prompt-size comparisons, not live model
ranking measurements, and no inference latency was measured for them.

| Baseline | Measured |
|---|---:|
| All titles in a prompt | 5,635 estimated tokens |
| Sequential per-session recall across every chat | 29.0 ms/query |
| All digests in a prompt | 12,446 estimated tokens |
| All entries in a prompt | 603,939 estimated tokens |

```sh
CARGO_TARGET_DIR=src-tauri/target-wt cargo test --manifest-path src-tauri/Cargo.toml \
  -p bridge-core --release --lib chat_search::eval::bench -- --ignored --nocapture
```

## Production funnel — Claude, tool-free, Haiku

200 chats / 20,000 entries, all 40 labelled queries, 18 of which run the model
stage. Same command both budgets:

```sh
CARGO_TARGET_DIR=src-tauri/target-wt cargo test --manifest-path src-tauri/Cargo.toml \
  -p bridge-core --release --lib chat_search::eval::live_fixture -- --exact --ignored --nocapture
```

Add `BRIDGE_CHAT_SEARCH_LIVE_WALL_SECONDS=30` for the diagnostic, and
`BRIDGE_CHAT_SEARCH_TURN_TIMING=1` to print each turn's wall time — that line
is what located the turn-cost problem above.

| Metric | 8 s (production) | 30 s (diagnostic) |
|---|---:|---:|
| Exact recall@4 | 15/15 | 15/15 |
| Vague recall@4 | 7/15 | 11/15 |
| Time + topic recall@4 | 10/10 | 10/10 |
| **Overall recall@4** | **32/40 (80%)** | **36/40 (90%)** |
| T2 median reported tokens | 1,544 | 3,324 |
| T2 maximum reported tokens | 3,404 | 11,842 |
| T2 median elapsed | 8,849 ms | 15,101 ms |
| T2 fallbacks | 12/18 | 2/18 |
| Usage rows tagged `chat_search` | 15 | 35 |
| Unsettled search sessions | 0 | 0 |
| Forest entries written by search | 0 | 0 |

Both runs simulated a shallow search, three seconds of typing, then Enter, and
a warm session was pre-started on the unsure shallow result. The 30-second run
is recall evidence only: it does not demonstrate the eight-second production
budget or the three-second latency target.

Two prompt changes were made and measured against this fixture. Telling the
model that the index matched only part of the memory, and that it should put
several synonyms in one `find_chats` call, moved the 30 s funnel from 35/40 to
36/40. A model may now also return its best guess alongside a lookup, so a run
the wall clock cuts short keeps the model's judgement instead of falling back
to the index's decoys; Haiku does not currently use that shape, so it is
inert but correct if a model does.

## Codex CLI is not a valid latency proxy

Codex CLI 0.157.1, `gpt-6-luna`, low effort. Every one of the 18 deep searches
fell back, with a median elapsed of 8,024 ms — the wall clock exactly — because
**one `codex exec` turn costs 6,533 ms and 21,829 tokens**. Two turns cannot
fit any budget a user would wait for, so a "Codex at 8 s" figure measures CLI
startup, not search behaviour. It is kept as an independent check that the
funnel runs on a second transport, not as latency evidence.

```sh
BRIDGE_CHAT_SEARCH_LIVE_HARNESS=codex CARGO_TARGET_DIR=src-tauri/target-wt \
  cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --release \
  --lib chat_search::eval::live_fixture -- --exact --ignored --nocapture
```

`BRIDGE_CHAT_SEARCH_LIVE_MODEL` selects another Codex model. The wall override
is confined to the ignored evaluator; production limits are unchanged.

## Real database smoke probe

A consistent SQLite read snapshot was backed up to a separate temporary
directory: 3,869,351,936 bytes in 16.8 seconds. An earlier backup without a
held read snapshot repeatedly restarted as the app wrote and was cancelled
after 180 seconds; its partial copy was removed. Opening and migrating the
successful copy took 28.6 seconds. The live app database was only opened
read-only for backup; the probe ran on the copy without booting or reaping any
of the live app's recorded adapter processes.

Three queries, three hidden sessions, 10 usage rows, zero unsettled sessions,
zero search forest entries. Those queries have no labelled expected answers, so
this is integration evidence, not a recall measurement. The production-Claude
real-database probe remains the one piece of live evidence still owed.

```sh
BRIDGE_CHAT_SEARCH_LIVE_DB=/tmp/copy/bridge.db CARGO_TARGET_DIR=src-tauri/target-wt \
  cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --release \
  --lib chat_search::eval::live -- --exact --ignored --nocapture
```

Set `BRIDGE_CHAT_SEARCH_LIVE_HARNESS=codex` for Codex, and supply queries with
`BRIDGE_CHAT_SEARCH_LIVE_QUERIES` separated by `|`. Never point it at the app's
live file: opening the store applies migrations and search writes its hidden
session and usage rows.

## Offline validation

- `bun run build` passed.
- Full `bun run test` passed: 229 frontend files / 2,856 tests, the sidecar,
  release-script and native menu checks, the Rust workspace, and doc tests.
  Exact command:
  `NODE_OPTIONS=--no-experimental-webstorage RUST_TEST_THREADS=8 CARGO_TARGET_DIR=src-tauri/target-wt bun run test`.
  `NODE_OPTIONS` is needed because Node 26's experimental web storage masks
  jsdom's `window.localStorage`, which fails unchanged updater tests.
- 73 `chat_search` tests passed, with 4 live/bench tests ignored. Added tests
  cover a partial index being announced to the model, a reply carrying a
  lookup and an answer together, and that guess surviving the wall clock.
- The release core suite passed three consecutive times at 2,762 passed / 15
  ignored. One earlier run of the same suite reported a single failure that did
  not reproduce in those three runs and was not a `chat_search` test; it is
  recorded here rather than described as clean, because a flaky test is not
  something this measurement can rule out:
  `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --release --lib -- --test-threads=8`.
- The remaining Rust workspace tests passed:
  `cargo test --manifest-path src-tauri/Cargo.toml --workspace --exclude bridge-core -- --test-threads=8`.