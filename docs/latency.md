# Send latency

A send crosses four hops. One measurement answers one question about the send.

| Hop | Boundary | What it includes |
| --- | --- | --- |
| 1 | Click to Bridge accepting the send | Composer work, RPCs, chat creation, preparation, and input delivery. |
| x | Bridge launching the harness through readiness | Bridge’s launch, prompt and context preparation, adapter startup, and the harness boot it waits for. Warm sends reuse the adapter. |
| y | Harness request to the first model output | Harness and provider execution, including model effort and queueing. |
| 2 | Output arriving at Bridge through paint | Frame normalization, forest persistence, publication, client delivery, and webview rendering. |

## Budgets

The release-mode benchmark drives the real core API and forest writer with an in-process adapter, a Git workspace, 1,500 visible chats, 50,000 Bridge-owned background runs, and 300,000 forest entries. Each round takes 60 samples after five warm-ups. The check has no model or RPC network dependency.

| Metric | Timed operation | p95 target | Measured p95 |
| --- | --- | ---: | ---: |
| New chat | Configured create, prepare, start, and submit | 300 ms | 175.33 ms |
| Warm send | Prepare and submit on the idle, already-launched chat | 50 ms | 1.07 ms |
| Forest entry | Handle one warm streaming frame and commit one forest entry in the Git workspace | 5 ms | 0.98 ms |
| App state | Build the app state and encode its JSON response | 150 ms | 78.32 ms |

The measured column records the median of three release-mode rounds on THINKBOOK (Linux x86-64, 2026-11-25), on `35cf95b6` with the benchmark and fixture version 1.

The Send latency job publishes the current p95 beside each target in the job summary. It compares the median of three round p95s for the merge base and head on one runner. A metric fails when its slowdown exceeds the larger of 20% of the base p95 or the noise floor. The initial floors are 1 ms for forest persistence and 5 ms for the other metrics. The summary flags absolute target overruns, including ones already present on the merge base. Download the `send-latency` artifact for both sets of raw reports and the compared revisions.

CI shares the workload and keeps each revision's composer entry points. A change to how the composer creates or starts a chat should update the corresponding calls in [`composer.rs`](../src-tauri/bridge-core/src/live_turn/tests/send_latency/composer.rs).

Run one round from the repository root. On THINKBOOK, prefix the command with `/home/ayushparihar/.local/bin/agent-run` when outside `ai.slice`.

```bash
cargo test --release --locked --manifest-path src-tauri/Cargo.toml -p bridge-core --lib \
  live_turn::tests::send_latency::measure -- --exact --ignored --nocapture --test-threads=1
```

## Latency ledger

The latency rows in the `events` table carry durations, counters, and bounded names. The report groups samples by harness and gives percentiles in seconds. A step can include a one-off wait, so check its sample count as well as its p95.

| Row | Producer and fields |
| --- | --- |
| `latency.client` | Webview milestones from the click to each RPC, the first frame, and the first painted output. Steps are offsets from the click. |
| `latency.rpc` | One daemon send-path RPC, broken down into dispatch, serialization, and write time, with response bytes. |
| `latency.rpc_window` | Per-minute counts, mean and maximum latency, and largest payload for state reads and slow requests. A window mean describes a batch of calls. |
| `latency.create_chat` | Configured chat creation's state-build and wire-conversion steps, with the snapshot session count. |
| `latency.launch` | Chat start's resolve, prompt, context, spawn, and persistence timings. |
| `latency.submit` | Input submission's gates, resume when needed, routing, preparation, and delivery. |
| `latency.harness` | Claude sidecar milestones from input on stdin through init, requesting, first byte, and first token. |

Read a live Bridge data directory with a Node version that provides `node:sqlite`, such as Node 22. The report opens the databases read-only. Its `bridge.persist_gap_same_frame` is a historical estimate from the gap between persisted entries belonging to one provider frame. Use it alongside the direct fixture measurement.

```bash
node scripts/latency-report.mjs --data-dir /path/to/bridge-data --since 2026-11-01
```

Use a running daemon in an isolated data directory for the socket experiment. The `state` mode reads the all-sessions compatibility method, `state/get_state`. The desktop benchmark above uses the app's `state/get_app_state` endpoint. The `chat` experiment runs real cold and warm turns and can incur provider charges.

```bash
node scripts/latency-experiment.mjs state --data-dir /path/to/scratch-bridge --n 5
node scripts/latency-experiment.mjs chat --data-dir /path/to/scratch-bridge --harness claude --n 3
```

On MACBOOK, get the PID of the running `bridged` process and sample it while reproducing a slow send.

```bash
sample <bridged-pid> 10 -file /tmp/bridge-latency-sample.txt
```

Find the dispatch and adapter reader threads. Look for time waiting on the database mutex, time inside SQLite queries or response encoding, and time waiting on child processes. Match the profile to the send, launch, and RPC rows from the same window.

## Performance rules

- Keep work in the send and per-frame paths bounded independently of session and history counts. Build app-wide state on the state-read path. The configured-create and explicit-start compatibility calls still return state while the composer migration in [#1040](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1040) is pending.
- Keep process creation, network calls, and filesystem scans outside the database guard. The streaming repository-stamp tests cover the warm path. The remaining stamp fallback and general assertion are tracked in [#1042](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1042) and [#1043](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1043).
- Release cache locks before filling them. Refresh once per key and serve the most recent known value while refreshing.
- Prefer read connections for reads independent of the writer’s uncommitted state. Keep forest persistence and publication in order.
- Measure the same workload before and after a performance change. Include the synthetic comparison and use the live ledger to explain which hop changed.
