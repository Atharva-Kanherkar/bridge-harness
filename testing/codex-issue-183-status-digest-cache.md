# codex/issue-183-status-digest-cache — Test Contract

Closes #183.

## Scope And Stated Assumptions

- The read path (`ManagedPayloadStore::status`) may serve a content digest it has
  already verified in this process. The write paths — `install`, `repair`, and
  `verify_existing_installation` — always digest the full tree and never consult a
  cache. They call `payload_tree_digest` directly today and continue to.
- The cache is **in-process memory only**. No new on-disk artifact.
  - An on-disk witness was rejected on two grounds. `ensure_directory_is_bridge_shaped`
    refuses an installation root containing anything other than `payload/` and
    `receipt.json`, so a sidecar would have to widen a security guard whose stated
    purpose is to fail closed on foreign content. And a persisted witness is
    forgeable by anything that can already write inside the managed root, which is
    precisely the actor a drift check exists to catch.
  - #183's acceptance is "a *repeat* list does no full tree walk", which process
    memory satisfies. The first status read per process still digests in full.
- The cache stores a *stat witness* of a tree that was verified to match its
  receipt. It is never a source of truth for what the payload contains.
- Ownership, symlink, and shape checks are unaffected: the fast path still walks
  the tree with every existing check in `collect_tree_entries`, and only skips
  reading file *bytes*.
- Out of scope: #181's backgrounding, #182's coordinator wiring, any protocol or
  UI change, any change to the digest format or the receipt schema. The wire
  payload of `list_managed_agents` and `inspect_managed_agent` is byte-identical
  before and after.

## Functional Behavior

### The redundant digests, which are the larger half of the cost

`status_of` digests twice per agent: once via `store.status()` directly, and again
via `resolution_of` → `managed_runtime::resolve_runtime`, which calls
`store.status()` at its second branch. `inspect_managed_agent` then adds a third
direct `store.status()` for the receipt and a fourth through `external_candidate`
→ `resolution_of`. Measured against the code on `main`:

| call | full tree digests before | after |
|---|---|---|
| `list_managed_agents`, 3 agents installed | 6 | 3 first call, 0 thereafter |
| `inspect_managed_agent`, 1 agent | 4 | 1 first call, 0 thereafter |

The "after" first-call numbers come from deduplication alone and hold even with the
cache disabled. This is corrected from #183's issue body, which said three and one.

### The stat witness

- Computed from the same `collect_tree_entries` walk the digest already performs,
  so the witness cannot cover a different entry set than the digest does.
- Commits to, per entry: canonical `/`-joined relative path, whether it is a
  directory or a file, file length, and mtime. On Unix it additionally commits to
  the inode, so a file replaced by rename is caught even if its mtime is forged to
  the old value.
- If any entry's mtime cannot be read, no witness is produced and the caller takes
  the full-digest path. Absence of a witness always means "verify properly".
- Keyed by installation root path *and* the receipt's `integritySha256`, so a
  reinstall at a different version can never hit an earlier entry.
- Recorded **only** when the full digest was computed and matched the receipt. A
  drifted tree is never cached, so drift is re-detected on every call until it is
  repaired.

### What the fast path cannot catch, stated plainly

A modification that leaves path, kind, length, mtime, and inode all unchanged is
invisible to the witness, and `status` will report `Installed`. Achieving that
requires write access inside Bridge's managed root plus deliberately restoring
metadata. It is not reachable by the accidental drift this check exists to find —
an editor, a `npm install`, a partial copy, a truncation — all of which change
length or mtime. `install` and `repair` still read every byte, so a payload is
fully verified whenever Bridge writes it.

## Unit Tests

In `bridge_core::managed_payload`:

- `a_repeat_status_read_does_not_reread_file_bytes` — install a payload, call
  `status` twice, assert the second call reads zero payload bytes. Byte reads are
  counted by a test-only counter incremented in `stream_file_into`, so the
  assertion observes the actual I/O rather than a timing proxy.
- `a_touched_payload_file_is_still_reported_as_drift` — install, then rewrite a
  file inside `payload/` with different content, assert `Repairable {
  IntegrityDrift }`. Run it both with a cold cache and with a warm one, because
  the warm case is the one a cache can break.
- `a_same_length_content_change_is_caught_by_mtime` — overwrite a file with
  different bytes of identical length, assert drift is still reported.
- `a_drifted_tree_is_never_cached` — drift a payload, call `status` twice, assert
  both calls report `Repairable` and both read bytes.
- `a_symlink_planted_in_a_payload_is_caught_on_the_fast_path` — warm the cache,
  replace a payload file with a symlink, assert `Repairable` rather than
  `Installed`. This is the check that proves the fast path still walks the tree.
- `a_reinstall_at_a_new_version_does_not_hit_the_previous_witness` — install,
  status, uninstall, install a different version, assert the new status is
  computed from the new tree and reports the new receipt.
- `the_stat_witness_ignores_directory_mtime_churn` — reading a directory can
  update its atime but must not invalidate the witness; assert a repeat status is
  still a fast path after a plain read of the tree.

In `bridge_core::managed_agents`:

- `inspect_digests_a_tree_once` — assert `inspect_managed_agent` performs exactly
  one payload status computation, via the same byte-read counter.
- `list_digests_each_agent_once` — assert three installed agents produce three
  status computations, not six.

## Integration / Functional Tests

- `bridge-core/tests/managed_agents_live.rs` (opt-in, `#[ignore]`) must still pass
  unchanged against real vendor payloads, including its existing assertion that the
  digest is stable across repeated reads of claude's 5552-file tree. That test is
  the one that would expose a witness which disagrees with the digest on a real
  tree rather than a synthetic one.
- The full `cargo test -p bridge-core` suite: 573 passing on `main` at `aded8ad`,
  and no existing test may change behavior. Any existing test that starts passing
  for a new reason is treated as a regression to investigate, not a win.

## Smoke Tests

- `cargo clippy -p bridge-core --all-targets` clean.
- `scripts/check-builtin-adapters.sh` green.
- `cargo test -p bridge-protocol` and the protocol gates green, confirming no wire
  change leaked in.

## E2E Tests

N/A for this change — no protocol or UI surface moves, so there is no new user
journey. The desktop path is covered by the existing live test plus the unchanged
wire payload assertion.

## Manual Tests

Timing proof, run in the worktree:

```
cargo test -p bridge-core --lib managed_payload -- --nocapture
```

And the measured claim for the PR body: a bench-style test that installs a
synthetic 4000-file payload, times the first `status` and the second, and asserts
the second is at least an order of magnitude cheaper in bytes read. Bytes read,
not wall clock — wall clock on a loaded machine is not a stable assertion, and I
have previously reported a timing number that the fixture itself caused.
