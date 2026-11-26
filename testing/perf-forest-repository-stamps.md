# Forest repository stamp test contract

Issue [#1042](https://github.com/Atharva-Kanherkar/bridge-harness/issues/1042) removes Git from locked forest persistence while keeping exact observations at return points.

- A fresh boundary reflects an edit made outside Bridge, including within the old cache reuse window. A clean change of commit still causes divergence from the prior boundary.
- A cold or invalidated cache lets mid-turn entries persist without starting Git on the writer thread. File-change bursts coalesce. Worktrees have independent caches, and an older background scan cannot overwrite a newer boundary observation.
- Checkpoint, compaction, and retained summary stamps describe the same worktree observation. The transaction still writes the group atomically.
- A mid-turn head uses the nearest stamped boundary on its active parent chain, including when the ancestor is outside the snapshot page. An unstamped boundary and an unavailable repository report unknown. Rewind and fork still read the current worktree.
- Private scratch chats keep their ancestor-repository guard. An unborn repository still skips the status scan. Repository markers stay out of model context.

## Benchmark

Use the ignored `perf_bench::perf_bench_concurrent_chat_streams` test with a shared worktree and again with four separate worktrees. Give each worktree one initial commit and 8,000 untracked files in 80 directories. Use the same fixtures and build profile before and after. Report preparation, mutex wait, mutex hold, total persistence, and Git starts under the guard. Use `scripts/latency-report.mjs` on the measured install to verify a p95 same-frame persistence gap of at most 5 ms.

### THINKBOOK fixture results

Each run wrote 480 entries across 12 chats. All times are milliseconds at p95.

| Worktrees | Build | Git starts under mutex | Preparation | Mutex wait | Mutex hold | Total |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Before | 74 | 10.40 | 25.84 | 9.38 | 32.35 |
| 1 | After | 0 | 43.79 | 6.12 | 2.81 | 45.61 |
| 4 | Before | 84 | 22.00 | 35.08 | 16.32 | 48.22 |
| 4 | After | 0 | 40.10 | 7.61 | 2.39 | 42.98 |

After the change, mid-turn p95 including mutex wait was 7.10 ms for one worktree and 8.32 ms for four. Subsequent boundary writes took 53.20 ms and 55.71 ms at p95, including the fresh Git sample. The same-frame gap on the measured install remains to be checked.
