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
  UI change, any change to the digest format or the receipt schema.
- The wire payload of `list_managed_agents` and `inspect_managed_agent` is
  unchanged in the steady state, and there is one deliberate difference outside it.
  Resolution previously re-read the payload, so a tree changing between the two
  reads could produce a status and a `backing` describing different observations.
  They now come from one snapshot. That is strictly more consistent, and identical
  whenever the tree is not being mutated mid-call — which is the only case the old
  code could differ in.

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

### The freshness rule

A witness match is not sufficient on its own. A hit additionally requires every
file's mtime to be strictly older than the moment the recorded verification *began*.

Without that, a same-length overwrite landing inside a single mtime tick would be
invisible, because the witness it produced would be identical to the one recorded.
Filesystem timestamp granularity is not guaranteed to be fine — HFS+ is a
one-second example — so this is not a theoretical window. Reading the clock before
the walk rather than after also means a file modified *during* a read can never be
older than the recorded time, so a torn read is never trusted on the next call.

Cost: one extra full digest when a status immediately follows a write. Clock skew
moving backwards degrades to always-digest, which is the safe direction.

### What the fast path cannot catch, stated plainly

A modification that leaves path, kind, length, inode, and mtime all unchanged *and*
whose mtime predates the last verification is invisible to the witness, and `status`
will report `Installed`. Reaching that requires write access inside Bridge's managed
root plus deliberately backdating metadata — and the backdating is itself caught,
because the witness commits to mtime, so the surviving case is narrower still: an
overwrite that reproduces the original mtime exactly.

It is not reachable by the accidental drift this check exists to find. An editor, an
`npm install`, a partial copy, a truncation — all change length or mtime. `install`
and `repair` still read every byte, so a payload is fully verified whenever Bridge
writes it.

## Unit Tests

Two test-only counters, both keyed by payload root so each test observes only its
own tempdir and the default parallel runner needs no serialization. A single global
counter would have raced every other test that touches a payload.

- `FULL_DIGESTS` / `full_digests_of` — times a tree's bytes were read in full.
- `TREE_WALKS` / `tree_walks_of` — times a tree was walked, cache hit or not.

The two are separate because the two costs are independent, and conflating them
hides a regression: with the cache in place, a *redundant re-read* is a cache hit
and adds no digest, so a digest-only assertion passes even with the duplicate reads
restored. Walks are the metric deduplication moves; digests are the metric the
cache moves.

In `bridge_core::managed_payload`:

- `a_repeat_status_read_does_not_digest_the_tree_again` — five repeat reads add
  nothing to the digest count.
- `a_touched_payload_file_is_still_reported_as_drift_with_a_warm_cache` — drift
  after warming, which is the case a cache can break.
- `a_same_length_content_change_is_still_caught` — identical length, different
  bytes.
- `a_backdated_in_place_overwrite_is_still_caught` — the one case the freshness
  rule cannot see, and therefore the only test that makes mtime-in-the-witness
  load-bearing: same inode, same length, mtime moved *backwards*. The test asserts
  the evasion is really set up before asserting it fails, so it cannot pass because
  some other mechanism fired.
- `a_drifted_tree_is_never_cached` — three drifted reads cost three digests.
- `a_symlink_planted_in_a_payload_is_caught_even_with_a_warm_cache` — proves the
  fast path still walks.
- `a_tree_reinstalled_at_the_same_path_is_read_in_full_again`.
- `reads_between_status_calls_do_not_force_a_redigest`.

In `bridge_core::managed_agents`:

- `list_and_inspect_read_each_payload_once` — three installed built-ins produce
  three walks and three digests for the first list, three walks and zero digests
  for a repeat, and one walk for an inspect. Asserts the wire payload is identical
  between the two lists, which is what makes this a pure performance change.

### Mutation verification

Every test above must be killed by at least one deliberate break. Recorded results,
all confirmed:

| mutation | kills |
|---|---|
| cache never consulted | the two cache-effectiveness tests |
| cache hit ignores the witness | both same-tree drift tests |
| witness drops mtime | the backdated-overwrite test |
| cache hit returns before the walk | four tests, including the symlink one |
| `resolve_runtime` re-reads the store | the agents test, 3 walks → 6 |
| `inspect` re-reads for its receipt | the agents test, 1 walk → 2 |

Two findings from this pass are recorded rather than papered over:

- Removing any of the three `forget_verified_payloads` calls kills nothing. A
  harmful stale entry is unreachable — installation paths are content-addressed
  through `installation_id`, and a hit re-checks the receipt integrity. They are
  kept as defence for a future where identity stops deriving from content, and the
  code says so rather than implying they are load-bearing.
- A witness committing to atime also kills nothing here, because this filesystem
  does not update atime on read. The test that would have covered it was renamed to
  claim only what it proves.

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
cargo test -p bridge-core --lib managed_payload
```

No wall-clock assertion anywhere. Every claim in this contract is expressed as a
count of walks or digests, because a timing number on a loaded machine is not a
stable assertion — and a previous measurement in this epic was reported as a win
when the fixture itself had caused it.

The real-tree evidence is the opt-in live test, which exercises claude's 5552-file
payload rather than a synthetic one:

```
cargo test -p bridge-core --test managed_agents_live -- --ignored --nocapture
```
