# fix/orphan-reaping-and-bounded-retention — Test Contract

Addresses the measurable core of the resource blow-up investigation: orphaned
`opencode serve` processes surviving supervisor death, the unbounded SSE event
queue, forever-growing history snapshots, and the 3-second full-history poll.

Live baseline captured on this machine on 2026-08-20 before any change:

```text
opencode_processes=145  orphaned_ppid1=145  total_rss_mib=3917.7
orphan_aggregate_pcpu=84.0%   (persisting after Bridge.app quit)
history snapshots: 1105 files / 2.8 GiB; app-data total 3.0 GiB
```

## Scope And Stated Assumptions

In scope, in dependency order:

1. **Durable launch ledger** — every `opencode serve` spawn (session and
   discovery/control) writes a ledger file under `<data_dir>/process-ledger/`
   recording child pid + identity, supervisor pid + identity, kind, and cwd
   before the child is used; the file is removed on verified stop. Boot
   recovery kills only entries whose recorded supervisor is dead AND whose
   child identity (`ps lstart+comm`) exactly matches. PID reuse or a live
   supervisor means no kill, ever.
2. **Parent-death watchdog** — unix spawns are wrapped in a `sh` monitor that
   polls its own PPID and terminates the wrapped process group when the
   supervisor dies. This covers SIGKILL, aborted `cargo test` binaries, and
   forced dev restarts, none of which run Rust destructors.
3. **Legacy orphan sweep** — at daemon boot, processes matching the exact
   Bridge spawn shape (`opencode serve --hostname 127.0.0.1 --port <n>`),
   re-parented to PID 1, and running from a Bridge checkout (`/src-tauri` in
   cwd) are terminated and recorded as recovery evidence. Disabled by
   `BRIDGE_SKIP_LEGACY_ORPHAN_SWEEP=1`. This is what reclaims the existing
   3.9 GiB / 0.84 cores; the ledger + watchdog prevent recurrence.
4. **Bounded event transport** — the per-session unbounded `mpsc::channel` is
   replaced by a bounded queue with an item cap and a byte budget. Transient
   frames (`message.part.delta`) are droppable once over budget, counted, and
   recovered losslessly from the terminal `message.part.updated` frame.
   Durable frames (lifecycle, approvals, errors, completions, usage) are never
   dropped; over the hard cap the producer blocks, which back-pressures the
   SSE socket instead of buffering unboundedly.
5. **Normalization state cleanup** — adapter `streams` maps drop a provider
   session's entry when its runtime ends.
6. **Snapshot hygiene** — snapshot hashing streams from a buffered reader
   (constant memory in DB size); a retention policy (newest 8, plus one per
   day for 7 days) prunes only valid `.sqlite` + `.manifest.json` pairs; the
   boot export is skipped when the newest snapshot is younger than the
   15-minute cadence; health reports snapshot count and total bytes.
7. **Forest digest polling** — a new `sessions/get_session_forest_digest`
   method returns a cheap change token composed from existing monotonic
   columns. The 3s poll fetches the digest and only fetches the full snapshot
   when it changes (with a forced full refresh every 10th poll as the safety
   net for external git state). `forestSnapshotKey`'s whole-snapshot
   `JSON.stringify` and both stringifies in `mergeForestSnapshot` are removed;
   entry identity uses length + last sequence, valid because the forest is
   append-only.
8. **Frontend byte bounds** — live agent events get a total byte budget and a
   per-item merged-text cap alongside the 2,000-item cap; terminal scrollback
   becomes a global-budget LRU instead of a per-workspace-forever map.

Explicitly deferred, with reasons:

- **Single shared OpenCode server per daemon.** An architectural change to
  session multiplexing that needs live multi-session validation; the ledger +
  watchdog close the orphan-accumulation hole the fan-out causes today.
- **Cursor-paginated UI history and conversation virtualization.** Belongs
  with the lazy-replay work already tracked separately; the digest fix removes
  the recurring cost (poll churn), not the one-time cost of opening a huge
  session.
- **Scheduler memory budgets.** Depends on the diagnostics surface introduced
  here; gating launch decisions on it is its own change.

Assumption: killing a process is only ever allowed when (a) the recorded
child identity matches the live process exactly and its recorded supervisor is
dead, or (b) the legacy matcher (argv shape + PPID 1 + Bridge cwd) holds. Both
fail closed: any mismatch or lookup failure means no kill.

## Functional Behavior

- Spawning a session or discovery server writes a ledger entry before first
  use; a normal stop removes it; the entry survives `kill -9` of the
  supervisor.
- Booting after supervisor death terminates exactly the ledgered children
  whose identity matches; mismatched or already-dead entries are cleared
  without a kill; entries whose supervisor is still alive are left untouched.
- Killing the supervisor with SIGKILL causes the watchdog to terminate the
  wrapped child within ~10 seconds without any restart.
- A stalled consumer cannot grow the event queue past its configured byte and
  item budgets; durable frames all arrive once the consumer resumes; dropped
  transient deltas are counted and the final message text is still complete.
- A session runtime ending removes its provider-session entry from the
  adapter's normalization map.
- Snapshot export produces byte-identical manifests to the old code (same
  sha256 for the same file); retention keeps newest 8 + 7 dailies and never
  deletes an unpaired or foreign file; health reports count and bytes.
- With an idle selected session, successive polls transfer only the digest
  (tens of bytes) instead of the full snapshot; any entry append, usage row,
  lease change, or head move changes the digest.

## Unit Tests

Rust (bridge-core):

- `process_ledger`: record → file exists with all fields; clear → gone;
  recovery kills a dead-supervisor + identity-matched `sleep` fixture (group
  killed, evidence event recorded); refuses on identity mismatch; skips when
  the recorded supervisor is alive; tolerates corrupt/foreign files.
- legacy sweep matcher: accepts the exact observed argv/cwd shapes, rejects
  near-misses (extra args, non-src-tauri cwd, live parent).
- bounded queue: flood of transient frames stays under byte+item budget with
  drop counters incrementing; durable frames are never dropped and arrive in
  order after a stalled consumer resumes; producer blocks at the hard cap and
  unblocks on drain; high-water mark reported.
- snapshot: streamed hash equals `Sha256::digest(fs::read(...))` for the same
  file; prune keeps the policy set and deletes nothing without a valid pair;
  boot export skips when the newest snapshot is fresh.
- digest: stable across repeated calls on an unchanged 5k-entry session;
  changes on entry append, usage append, lease update, head move; cheap (no
  entry payloads loaded).
- streams cleanup: forget removes the provider key; the `"default"` fallback
  key is never evicted per-session.

Vitest (src):

- `agentEvents`: total byte budget enforced (oldest evicted), per-item merged
  text capped keeping the tail, 2,000-item cap unchanged.
- `forest`: merge keeps entry identity via length + last sequence; no
  `JSON.stringify` of snapshots anywhere in src/forest.ts.
- terminal scrollback LRU: global budget enforced, most-recent survives,
  eviction on insert.

## Integration / Functional Tests

- `bridged` daemon test: boot with a seeded ledger entry for a dead-supervisor
  `sleep` fixture in its own process group → process is gone after boot and
  the ledger entry removed. Matches the existing killed-daemon recovery test
  pattern.
- Watchdog end-to-end: an intermediate supervisor process spawns a
  watchdog-wrapped `sleep`, is SIGKILLed, and the wrapped `sleep` exits within
  the watchdog interval; with the supervisor alive the child stays up.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar, vitest, cargo workspace).
- `bridged --data-dir <scratch> --health-addr none` boots, runs the sweep and
  ledger recovery without error, and shuts down cleanly on SIGTERM.

## E2E Tests

N/A — no UI journey changes; the digest path is covered by unit tests plus the
manual verification below.

## Manual / Measurement Verification

Baseline (already captured above). After implementation, on this machine:

```bash
# orphan census before/after booting the fixed daemon against a scratch dir
ps -axo pid,ppid,rss,pcpu,command | grep 'opencode serve' | grep -v grep | \
  awk '{c++; r+=$3; p+=$4} END {printf "count=%d rss_mib=%.1f pcpu=%.1f%%\n", c, r/1024, p}'
```

Expected: count drops to 0 (or only entries with live parents), reclaiming
~3.9 GiB RSS and ~0.84 cores of steady CPU burn.

```bash
# poll cost: ignored bench prints serialized bytes + wall time for 100 polls
# of a 5k-entry session, full snapshot vs digest
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core \
  forest_digest_poll_cost -- --ignored --nocapture
```

Expected: per-poll payload goes from O(history) (megabytes) to tens of bytes;
poll wall time drops by >10x on the seeded session.

Snapshot retention on the real store is applied on first normal app run; the
policy math for today's dir (1105 files / 2.8 GiB) retains ≤15 files.

## Local refund checkpoint

Before the PR: ledger record/clear/recover matrix, watchdog kill-on-parent
-death, bounded-queue stall + losslessness, snapshot hash/prune/skip, digest
stability + invalidation, frontend byte caps, full `bun run build` +
`bun run test`, and `git diff --check`.
