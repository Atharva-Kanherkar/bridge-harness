# Worktree lifecycle

Bridge cuts a Git worktree for an isolated chat, for every isolated worker, and
for a pull request checked out for review. Creating one is cheap; forgetting one
used to be expensive, because agents ran builds *inside* these checkouts and a
populated `target/` and `node_modules` turned a 30 MB checkout into a 4 GB one.

Two mechanisms, in the order they matter. **Build caches** stop the cost
accruing: the expensive directories live once per repository, outside every
checkout. **The inventory and its retention policy** manage what is left.

This describes the inventory that records them and the retention policy that
reclaims them. It does not change the three hierarchies: forking a conversation
still does not undo filesystem changes, and ending a session still does not
discard a worktree. Reclaiming is a decision the worktree coordinator makes
against the filesystem, never a side effect of a conversation moving — and,
because branch refs survive, never a decision that loses a session its project.

## Build caches

Every agent session process, and every verification command, runs with
`CARGO_TARGET_DIR`, `BUN_INSTALL_CACHE_DIR` and `npm_config_cache` pointed at
one cache per repository under `build-caches/` in the data directory. Keyed on
the repository's main worktree, so every checkout of a project shares one cache
and a second worker starts warm — dependencies dominate a build directory and
they are identical across branches.

Two costs are paid knowingly. Concurrent builds of one repository serialize on
cargo's lock rather than building in parallel, which is the right trade against
a multi-gigabyte target directory per worker (`max_concurrent_workers` is 2).
And switching branches rebuilds changed crates, while unchanged dependencies —
the bulk — are reused.

A variable already set in Bridge's environment is never overridden: an operator
who pointed a cache somewhere deliberately outranks this. A process with no
resolvable repository gets no cache rather than a wrong shared one. `CARGO_HOME`
is deliberately not redirected — it holds registry credentials, which makes
relocating it a security decision rather than a disk one.

**Read-only sandboxed workers are excluded.** Their seatbelt permits writes only
under their own output directory, and widening that to reach a shared cache
would trade away part of the read-only guarantee to speed up the one class of
worker that is not supposed to be building.

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

Symlinks are skipped rather than followed. `is_dir` follows them, so a symlink
dropped into a layout slot — which anything with write access to its own
checkout can create as a sibling — would otherwise be inventoried as a Bridge
checkout at its *resolved* path, outside the namespace entirely. Namespace
containment is re-checked when the sweep selects a candidate and again
immediately before deletion, so a row that names a path outside the namespace,
however it came to be written, is never removed.

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

"In use" is not a question status alone can answer. A chat whose provider is up
but has no turn in flight sits at `ready`, so a status-only test would call a
live adapter's checkout idle. `sessions.adapter_pid` is the durable claim that a
real process owns the session, and boot recovery clears it for every process
that is actually gone — so a live adapter always retains its checkout, and a
stale `ready` row left by a crashed run does not pin one forever.

The classification that authorizes a deletion is re-run in full immediately
before it, against facts read at that moment. This is not covered by the git
refusal underneath: `git worktree remove` needs `--force` only for a dirty or
locked tree, not for a clean one carrying a commit that exists nowhere else. A
commit made between the scan and the deletion would otherwise go with the
directory.

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

The sweep reduces to *strictly below* each cap rather than merely to it. The
creation gate refuses at `>= max_per_repo`, so stopping at equality left a
repository permanently full: the tick reclaimed nothing and every queued
delegation waited on a TTL with no reason to expire. Clearing a cap has to leave
room for the work the cap is blocking.

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
checkout. All four methods are reachable from the CLI without a bespoke
subcommand — `bridge exec --json --method worktrees/list_worktrees` — because
`exec` forwards any registry method.

## Scratch branches

A worker's `<task>-worker-<uuid>` branch is deleted once its checkout is gone and
its commits are provably elsewhere: either the branch never moved off its base,
or its tip is already reachable from the task branch that adopted it. Thirty such
refs were sitting in one repository when this was written, every one belonging to
a worker that had long since settled.

The branch must be one Bridge minted, and that is established by
*reconstructing* the name `prepare_isolated_worker` would have produced — from
the base branch and the session slug — rather than by matching on `-worker-`. A
task branch may itself be called `bridge/foo`, so no prefix rule can tell an
orchestrator branch from a worker branch cut off it. `git branch -d` refuses
anything still unmerged, so a wrong answer fails closed.

Both paths that reclaim a checkout share this: settlement, and the retention
sweep once it settles an empty binding.

## Reclaiming on request

`worktrees/reclaim_worktree` takes one checkout; `worktrees/sweep_worktrees`
runs a full pass. A refusal is a *result*, not an error — "no, and here is why"
is the useful answer for a button — so at-risk, retained and unverifiable
checkouts come back with their reason, and only an unknown id is an error. Both
go through the same removal path as the sweep and inherit its deletion-time
re-classification.

One deliberate difference: an explicit request may reclaim a checkout whose every
commit is already on a remote, which the unattended sweep retains. The difference
is who is asking, and whether they have been told what they are discarding.

Settings → Storage renders this: total bytes against the cap, a per-repository
rollup, and a row per checkout. A row either offers Reclaim or states in words
why nothing may touch it — no disabled button with a tooltip, because "at risk"
without "uncommitted changes" tells a person nothing they can act on.

## Archiving a chat

`sessions/archive_chat` files one conversation away and reclaims the checkout
*that chat owns*. It is deliberately not `archive_workspace`, which deletes every
session in a workspace: a workspace holds many chats, so a per-chat button wired
to it would destroy hundreds of unrelated conversations to reclaim one directory.

History is kept — the session row, its forest entries and its evidence all
survive, and the projection simply stops listing the chat and its descendants. A
checkout that cannot be proven expendable is kept rather than blocking the
archive, and the reason is reported: putting a conversation away should not
require first resolving its uncommitted work. A running chat is refused until it
is stopped.

This is the affordance whose absence caused the accumulation in the first place.

Branch refs are never deleted by the retention sweep. Only checkouts are
reclaimed —
which is what makes reclaiming reversible. `start_chat` takes `sessions.cwd`
verbatim and only creates the directory, so a resumed chat whose worktree had
been collected would otherwise start in an empty non-repository where its
project used to be. `restore_if_reclaimed` cuts the checkout again from the
recorded branch before the session starts, the same restore
`worktree_coordinator` already performs for a pull-request checkout whose tree
was reclaimed. It refuses to recreate anything Bridge did not cut.

Related: [`delegation-policy.md`](delegation-policy.md) for the worker adoption
lifecycle, [`session-forest.md`](session-forest.md) for the three hierarchies,
[`local-history.md`](local-history.md) for the equivalent retention policy over
history snapshots.
