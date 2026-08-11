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
  - #183's acceptance was written as "a repeat list does no full tree *walk*". That
    is not what this delivers and not what the tests assert. The walk always runs —
    it is what keeps the symlink and shape checks live — and what a repeat read
    avoids is **re-hashing the bytes**. The acceptance is therefore read as "no
    repeat full byte digest", which is what `full_digests_of` measures. Corrected on
    the issue rather than left ambiguous between the two readings.
  - The first status read per process still digests in full, and so does any read
    within the granularity margin of the payload being written.
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
- Stored in a map keyed by the installation's payload root. The receipt's
  `integritySha256` is a *field* on the entry, re-checked on every hit, not part of
  the key — so one path holds at most one entry and a later insert replaces it. A
  reinstall at a different version cannot hit an earlier entry because the integrity
  check rejects it, and because a different version is a different path anyway.
- Recorded **only** when the full digest was computed and matched the receipt. A
  drifted tree is never cached, so drift is re-detected on every call until it is
  repaired.

### The freshness rule

A witness match is not sufficient on its own, and a bare "older than the
verification" comparison is not sufficient either. That was the first version of
this rule and it was wrong: mtimes are truncated to the filesystem's granularity
`g`, while the verification timestamp is nanosecond wall clock, so the two are not
directly comparable.

Masking requires a post-verification write whose truncated mtime equals the one
already in the witness — `bucket(write) == bucket(previous write)`. Because the
write happens at or after the verification, that is possible exactly when the
verification itself fell inside the previous write's bucket. On a one-second
filesystem: a file written at `S.1` carries mtime `S.0`, a verification beginning at
`S.3` records `S.3`, and a same-length in-place overwrite at `S.7` truncates to
`S.0` again — witness unchanged, `S.0 < S.3` still true, hit served, drift masked.

So the rule is "did the verification happen at least one full bucket after the file
was written", not "is the file older than the verification". With `g` unknown at
runtime, a 2-second bound stands in for it: FAT records mtimes in 2-second steps and
HFS+ in 1-second steps, and APFS, ext4, NTFS and ZFS are all finer.

Reading the clock before the walk rather than after keeps the recorded time no later
than the bytes being read, so a file written during the walk cannot then appear to
predate the verification by a full bucket.

Cost: a payload verified within two seconds of being written is re-digested on the
next read. Next to a 15–32 second install, noise.

Stated limits, rather than claimed away:

- A filesystem with mtime granularity coarser than 2 seconds — some network mounts —
  is outside the bound, and the same-bucket window reopens there.
- The rule anchors on wall clock, because mtimes are wall clock and a monotonic
  clock cannot be compared to them. A system clock stepped backwards by more than
  the margin after an entry is recorded can put a later write back inside the
  recorded bucket. An earlier draft of this contract asserted backward skew was the
  safe direction; that was stated without working it through, and it is the
  dangerous one. Anything able to step the system clock can also write to the
  managed root directly, and `install` and `repair` still read every byte, so this
  is recorded as a bound on the cache rather than treated as a defence to build.

### What the fast path cannot catch, stated plainly

A modification that leaves path, kind, length, inode, and mtime all unchanged, and
whose mtime is more than the granularity margin older than the last verification, is
invisible to the witness, and `status` will report `Installed`. Reaching that
requires write access inside Bridge's managed root and an overwrite that reproduces
the original mtime exactly — backdating to any *other* value is caught, because the
witness commits to mtime.

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
- `a_touched_payload_file_is_caught_by_a_witness_mismatch` — drift with an entry
  already present, so the rejection path runs rather than a cold read. The rewrite
  changes length, so what rejects it is the witness; the name says so rather than
  claiming warm-hit coverage it does not have.
- `a_same_length_content_change_is_still_caught` — identical length, different
  bytes.
- `an_overwrite_inside_the_verification_mtime_bucket_is_still_caught` — the hole the
  first version of the freshness rule left, reproduced by setting mtimes explicitly
  so it does not depend on the host filesystem's real granularity.
- `the_freshness_rule_requires_a_full_bucket_not_merely_an_older_mtime` — the
  predicate alone, including the boundary and a saturating add that must fail closed.
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
| freshness reverts to a bare `newest < started_at` | the bucket-window test and the predicate test |
| granularity margin off by one at the boundary | the predicate test |
| cache never consulted | the two cache-effectiveness tests |
| cache hit ignores the witness | both same-tree drift tests |
| witness drops mtime | the backdated-overwrite test |
| cache hit returns before the walk | five tests, including the symlink one |
| `resolve_runtime` re-reads the store | the agents test, 3 walks → 6 |
| `inspect` re-reads for its receipt | the agents test, 1 walk → 2 |

A fixture note that matters for reading these: a freshly installed payload is
younger than the granularity margin, so it is deliberately *not* cacheable. Fixtures
age their trees an hour into the past to represent the steady state the cache exists
for. Ageing explicitly rather than sleeping keeps the suite fast and the intent
visible — and three tests failing when the margin was introduced is what surfaced
this, correctly.

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
