# Streaming UX validation

The implementation was started from main at `50722e56` with the contract in
[fix-streaming-ux.md](fix-streaming-ux.md) committed before implementation.

## Coverage

- Raw Claude block-stop replay completes thinking before answer deltas. Tests cover accumulated text, multiple blocks/messages, delayed snapshots, duplicate stops, stray late deltas and interrupted results.
- The built-in adapter fixture now includes real block indexes and a thinking stop. Its later snapshot corrects the same thought identity.
- Existing transcript adoption, interleaving, golden and rendering tests remain required. New structural tests cover 100, 1,000 and 5,000 historical entries with a live tail capped at 1,000, counting fallback work rather than relying only on timing.
- A mounted React test asserts that live-only updates do not repeat durable projection.
- Display tests cover first text, lifecycle boundaries, concurrent workers, background fallback, disposal and a 5,000-frame burst. Native lifecycle tests assert that earlier deltas precede immediate boundary delivery.
- Timing tests cover monotonic durations, optional metadata, coalescing, bounded retention and absence of diagnostics in persisted session entries.

## Verified checks (2026-09-07)

Environment: macOS 26.2 (25C56), Node 26.3.0, Bun 1.3.13. The branch is
based on main at `50722e56`; fetching main before these checks found no newer
base changes.

- `bun run build`: passed (TypeScript and Vite).
- `NODE_OPTIONS=--no-experimental-webstorage bun run test`: passed. Sidecar:
  44 passed, 1 authenticated-runtime test skipped. Frontend: 139 files,
  1,845 tests passed. Rust workspace: 2,233 passed, 13 ignored.
- The Node option disables Node 26's experimental Web Storage so jsdom supplies
  test storage. It does not skip tests or change application configuration.
- `git diff --check`: passed.

The self-review covered the complete branch diff and uncommitted implementation
against the locked contract. No separate worker review or successful full-app
performance verification is claimed. Build output includes the existing large
chunk warning; static React render tests emit `useLayoutEffect` warnings.

## Display budget

The intentional buffering budget is 32 ms: native IPC batches for at most 16 ms;
the webview flushes on its next animation frame or a 16 ms fallback timer.
Lifecycle events flush pending predecessors at both boundaries. The first visible
content in each session/turn flushes immediately on webview receipt. Concurrent
workers share frame batches after their first content.

This is a buffering policy, not a hard response-time guarantee. OS scheduling,
background timer throttling, IPC, database work and rendering can exceed it.
Queues remain bounded when a window cannot receive animation frames.

## Timing diagnostics

Launch the daemon/native process with `BRIDGE_STREAM_TIMING=1`. In the webview
inspector, call `window.bridgeStreamTiming.snapshot()` and
`window.bridgeStreamTiming.clear()`. The ring holds at most 512 content-free
samples; diagnostics are attached after persistence and never enter the forest.

Each sample has a frame ID and normalized-event ID, native database-wait,
normalization, persistence and receipt-to-publication durations, plus local
webview receipt, receipt-to-React-commit and receipt-to-paint-proxy durations.
The paint proxy is a second animation-frame opportunity, not a compositor trace.
Coalesced deltas retain the newest frame's sample. Hidden/unrendered events can
have no commit or paint measurement; evicted/coalesced samples can remain without
those measurements. These are diagnostic samples, not population percentiles.

Native `Instant` and webview `performance.now()` are separate monotonic clocks.
Do not subtract one from the other. Publication-to-webview transport is not
measured by these durations, so adding the two sides is not total end-to-end
latency. Provider time-to-first-token and generation rate require provider-side
measurements and must be reported separately.

## Native WebKit replay

The reproducible harness uses production transcript components, projection,
markdown and the display scheduler in a desktop WKWebView. It asserts that the
historical transcript rows actually mounted. It uses synthetic inputs, no
provider requests, no credentials and no application database.

```sh
bun run dev
swift scripts/benchmark-streaming-webkit.swift http://127.0.0.1:1420/testing/streaming-webkit.html
```

Keep the checkout unchanged while measuring: Vite reloads the page on edits.
The harness reports medians after three warmups and fifteen measured projection
runs, then 120 scheduled updates for each render workload. The rendered long
history has 5,000 entries; the worker workload interleaves four session streams.
It reports synchronous commit duration, a 16 ms event-loop timer's lateness and
pending queue size. Timer resolution can report sub-millisecond work as zero.

### Actual replay status (2026-09-07)

The corrected harness was run from this worktree on a dedicated Vite port;
port 1420 belonged to another checkout:

```sh
bun run dev -- --port 1431 --strictPort
swift scripts/benchmark-streaming-webkit.swift http://127.0.0.1:1431/testing/streaming-webkit.html
```

It reported `mounted 100 text` and `mounted 5000 text`, confirming the real
transcript-row assertion passed, then hit the native harness's 120-second
timeout before reporting final rendering results. This is an incomplete replay,
not a passing performance check. The remaining reasoning, code and worker
workloads were not reached. No final commit-duration or queue-drain measurements
are claimed. The worker workload, when run, schedules four session streams but
renders only the selected session; it does not measure four visible transcripts.

Earlier timings from the harness that omitted transcript rows are invalid and
must not be used. Investigating the corrected long-history timeout and completing
the replay are required before this draft is ready to merge. The timeout alone
does not establish whether the remaining cost is rendering, layout, development
instrumentation or another runtime effect.

## Remaining full-app validation

Synthetic WKWebView replay does not exercise actual composer typing, Stop RPC,
provider capture, full-app session switching or scroll anchoring. Those remain
manual checks in the contract. The native test suite covers Stop and compaction
suppression, but that is not a substitute for an interactive desktop trace.

Slow-Git contention remediation and its fixture remain tracked in
[Atharva-Kanherkar/bridge-harness#545](https://github.com/Atharva-Kanherkar/bridge-harness/issues/545).
This change preserves the existing database commit/publication order and adds
lock-wait visibility; it does not move repository work out of that lock. Keep
[the streaming issue](https://github.com/Atharva-Kanherkar/bridge-harness/issues/565)
open for the remaining full-app and backend validation.
