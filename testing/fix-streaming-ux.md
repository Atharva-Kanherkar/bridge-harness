# fix-streaming-ux — Test Contract

## Functional Behavior
- Claude raw thinking deltas followed by the matching block stop produce a completed thought with accumulated text before answer output.
- Thinking blocks have stable message/block identities; snapshots reconcile without duplicate/reopened thoughts. Multiple messages, blocks and interrupted streams retain text.
- Other providers retain legitimate reasoning/answer interleaving.
- Live updates reuse unchanged durable projection. Completed history and text-shadow merging avoid repeated whole-history fallback scans, preserving adoption, order and identity.
- Native IPC batching and frontend display scheduling have an explicit bounded latency budget. First visible content and lifecycle boundaries flush promptly in order; background windows and teardown cannot accumulate an unbounded queue.
- Correlated monotonic measurements distinguish provider-frame processing, database wait, publication, webview receipt, React commit and a paint proxy. Application measurements do not claim provider generation latency.
- Database transaction/publication ordering, Stop and compaction suppression remain intact. Repository lock restructuring and slow-Git remediation remain owned by the separately tracked backend work.

## Unit Tests
- Rust Claude normalizer: raw block stop, snapshot reconciliation, multiple blocks/messages, missing stop/interruption and empty/unknown blocks.
- Transcript reducer: named and unnamed adoption, thinking lifecycle, interleaving and reload.
- Merge: text shadow matching with duplicate texts, user messages and named/unnamed live items; preserve first matching anchor.
- Scheduler: first text, successive deltas, reasoning/message completion, tool/approval, Stop/error, session changes, background fallback, bounds and teardown.

## Integration / Functional Tests
- Raw Claude normalized completion can persist/replay with the same identity and full text.
- History scaling fixtures at 100/1,000/5,000 entries with a bounded live tail; structural work checks in addition to timings.
- Unchanged durable inputs are not reprojected by live-only renders.
- Timing metadata remains optional and does not alter ordering or expose message content.
- Run `bun run build` and `bun run test`; both must pass before PR creation.

## Smoke Tests
- Replay sustained text/reasoning, large code output and concurrent session streams; no increasing queue backlog.

## E2E Tests
- Exercise desktop WebKit with short/long transcripts, typing, Stop and scroll anchoring. Record actual environment and results; do not substitute headless Chromium timings for desktop evidence.

## Manual / cURL Tests
- In desktop Bridge, start a reasoning turn: thought settles when its block ends before the answer completes.
- Switch sessions during streaming, scroll upward, return to bottom and Stop; ensure stable ordering and responsive controls.
- Inspect correlated latency samples across receipt/commit/paint. Keep provider timing separate.
- Report unavailable desktop/manual checks explicitly; do not claim full issue completion without them.
