# Worktree lifecycle

Bridge cuts a Git worktree for an isolated chat, for every isolated worker, and
for a pull request checked out for review. Creating one is cheap; forgetting one
is not, because agents run builds *inside* these checkouts, and a populated
`target/` and `node_modules` turn a 30 MB checkout into a 4 GB one.

This describes the inventory that records them and the retention policy that
reclaims them. It does not change the three hierarchies: forking a conversation
still does not undo filesystem changes, and ending a session still does not
discard a worktree. Reclaiming is a decision the worktree coordinator makes
against the filesystem, never a side effect of a conversation moving.

## What exists

Every worktree Bridge creates gets a row in `worktrees`, keyed by its
canonicalized path, recording its kind, the repository it was cut from, the
session and workspace that own it, the commit it was cut from, when it was last
used, its measured size, and the last assessment of what may be done with it.

| Kind | Path | Branch | Owner |
| --- | --- | --- | --- |
| `orchestrator` | `worktrees/orchestrators/<workspace>/<session>` | `bridge/<workspace>-<session8>` | the chat session |
| `worker` | `worktrees/workers/<task>/<session>` | `<task-branch>-worker-<session>` | the worker session |
| `github` | `worktrees/github/pr-<n>-<branch>` | the PR head branch | the workspace node |

Orchestrator checkouts previously had no record of any kind — they were named
only by `sessions.cwd`, which no cleanup path consulted — so nothing could find
one, let alone reclaim it. That is the class this table exists for.

Paths are stored canonicalized. `git worktree list` reports resolved paths, so
on a machine where a worktree sits under a symlink (`/var` and `/tmp` on macOS, a
symlinked home or mount anywhere) the path Bridge recorded and the path git
reports are different strings for the same directory. Comparing them raw makes
Bridge file its own worktrees as somebody else's.

## Reconcile

A maintenance pass brings the inventory, git's registrations, and the filesystem
back into agreement. Drift is ordinary — a crash between creating a checkout and
writing its row, a directory deleted by hand, an out-of-band `git worktree
prune` — and what matters is that afterwards nothing is invisible.

- A row whose directory is gone becomes `removed`, and git's leftover
  registration is pruned.
- A directory under the namespace root with no row is adopted as `orphaned`,
  carrying whatever branch and head git reports.
- A directory git cannot read at all is adopted as `unverifiable`.
- A checkout of a repository Bridge knows that sits *outside* the namespace root
  is recorded `external`. It is reported and never reclaimed: a developer's own
  worktree is not Bridge's to collect.

## What may be reclaimed

Classification runs at deletion time, not when the inventory was scanned, and it
must *prove* a checkout is expendable. The dispositions:

| Disposition | Meaning | Swept |
| --- | --- | --- |
| `reclaimable` | Clean, and nothing landed past the commit it was cut from | yes, past its TTL or under cap pressure |
| `pushed_unmerged` | Clean, holds commits, all of them exist on a remote | only when the policy says `delete` |
| `at_risk` | Uncommitted changes, or commits that exist nowhere else | never |
| `retained` | A live session or worker is in it, or a worker's output there is still awaiting adopt-or-discard | never |
| `unverifiable` | Git cannot read it, so nothing about it can be established | never |

Every question that cannot be answered counts against deletion. A checkout whose
recorded base commit is unreadable, or whose remotes cannot be consulted, is
`at_risk` rather than assumed empty.

Idleness is measured from the owning session's own most recent recorded activity,
not from when the checkout was created — otherwise an isolated chat someone works
in daily would age out of its own TTL.

## Caps

Per repository: at most 12 Bridge worktrees and 10 GiB, with idle TTLs of 24
hours for a settled worker and 7 days for a chat or a PR checkout. Defaults live
in `WorktreeRetention`.

The sweep first collects expendable checkouts past their TTL. If a cap is still
breached it keeps taking expendable checkouts, least recently used first, even
inside their TTL — a cap is a promise about the machine. If the only checkouts
left are ones nothing may delete, the breach is *reported* (`over_budget_bytes`)
rather than forced. There is no `--force` path anywhere in this system.

Capacity is also part of routing. `child_worktrees_available` previously asked
only whether the namespace root had a parent directory — true for every path
that is not the filesystem root — so the policy engine's
`ChildWorktreeUnavailable` refusal could never fire. It now asks the inventory,
so a repository at its cap queues the delegation instead of cutting another
checkout, and the creation path refuses with a reason naming the budget.

## The stopped worker

A worker that *stops* never settles, so its binding stays `pending_adoption` and
every collector skips it. Being unsettled is not the same as holding work: when
the checkout is clean and nothing landed past its base commit, there is nothing
for anyone to adopt, and leaking a directory to protect an empty diff is a bad
trade. Those settle as `empty` and release. A worker that stopped holding real
changes — committed or uncommitted — keeps its checkout, and the decision stays
the user's.

## Where it runs

`live_turn::start_worktree_maintenance` runs one pass as soon as the host is
serving and then every 30 minutes. Deliberately not on the boot path, for the
reason the history-snapshot export documents: the pass shells out to git per
repository and measures directory sizes, and boot has to reach a bound socket
before the desktop shell's start deadline.

Reads and writes to the database happen under the shared mutex; classification,
size measurement, and every git invocation happen without it.

## Auditing

Every reclaim writes a `worktree.reclaimed` event naming the path, branch, head,
disposition, and bytes freed. A refusal writes `worktree.retained` with the
reason. `worktrees/list` and `worktrees/usage` expose the inventory and the
totals against the caps in force; both report the *last* assessment rather than
recomputing, since deciding a disposition costs several git invocations per
checkout.

Branch refs are never deleted by any of this. Only checkouts are reclaimed.

Related: [`delegation-policy.md`](delegation-policy.md) for the worker adoption
lifecycle, [`session-forest.md`](session-forest.md) for the three hierarchies,
[`local-history.md`](local-history.md) for the equivalent retention policy over
history snapshots.
