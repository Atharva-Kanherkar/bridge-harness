# Worker and storage audit

Audited from main at 4473707, then integrated the newer main before verification.

## Worker control plane

The original audit describes an older implementation. These mechanisms already
exist on main and are preserved, not replaced by this change:

| Original user problem | Current mechanism |
| --- | --- |
| Repeated paid result repairs after a quota error | Durable repair counter, reported-result gate and terminal adapter teardown |
| Quota exhaustion never triggers failover | Provider error observations, reset-aware cooldowns and one objective-bound fallback attempt |
| No reliable stop request | Typed stop request, process teardown, cancelled settlement and parent notification |
| Steers overwrite each other | Per-session queues and visible refusal paths |
| A valid result followed by chatter is lost | Result-bearing-message lookup and fixture-backed parsing |
| A stall looks like a generic failure | Host-authored failure classification |
| Cancelled means a red failure | Neutral cancellation in the shared worker status resolver |

The remaining user-facing gaps addressed here:

- Mission Control and worker detail now offer Stop without opening the session.
- Missing worker runtime data reads as unavailable, not successful or invisible.
- Harness and model are visible, with terminal elapsed time frozen.
- Retry refusals, quota events, substitutions, disabled harnesses and undelivered
  guidance are projected into expandable diagnostics, independent of prose activity.
- New launches record their routing explanation against the child session so the
  existing workspace ledger can return it to worker surfaces.
- Settings has a Workers page with workspace-local concurrency, per-turn limits,
  stall timeout, warm retention, automatic retries and provider-limit failover.
- The default harness is explicit. Automatic selection consults installed runtimes;
  role profiles and explicit route constraints still take precedence.
- Advanced routing and role profiles are reachable from Settings. Its existing
  exclusions restrict worker routing without disabling the harness for direct chats.

The hard limit of one automatic attempt per objective remains deliberate. The two
toggles permit or forbid an attempt; they do not multiply the shared budget.
Approval timeout, quota reset interpretation, strong-worker and capability budgets
remain host-owned safeguards, not controls on this page. Previously recorded route
decisions are not backfilled into the new diagnostic event.

## Worktrees and storage

- Storage now supports repository/status filters, search, size/idle/name ordering,
  per-repository cap warnings, measurement timestamps and inline errors.
- Cleanup asks for confirmation and explains which directory is removed. Backend
  deletion-time safety checks remain authoritative, including for stale UI rows.
- External checkouts remain read-only. Bridge does not acquire ownership by a user
  clicking a confirmation; this is the explicit policy choice for external trees.
- Worker worktree creation no longer holds the database mutex across Git or
  dependency copying. Existing leases still control write scope.
- Git observation commands have a 30-second wall-time limit, bounded output and
  process-group termination. Failed reconciliation observations are ledger events.
- Directory measurements have a five-second deadline and return incomplete rather
  than block their caller. A kernel-blocked filesystem thread cannot be forcibly
  cancelled portably; it exits after the syscall returns and sees its deadline.
- Identical tracked dependency manifests and lockfiles permit an independent
  copy-on-write node_modules seed. A mismatch, absent lockfile, unsupported clone
  or cross-filesystem failure leaves no partial installation at node_modules.
  The worker then uses the normal install workflow; Bridge does not execute project
  install scripts during worktree creation.

The source installation must already be valid for its manifests, just as it must
be for a build in the source checkout. Changing dependencies afterwards requires
the normal package-manager install in that checkout. No shared writable symlink or
hardlink tree is introduced.

The UI reports logical bytes, not unique APFS allocation. A clone shares data
blocks but du still charges those blocks to both files; summing du cannot prove
physical savings. There is no universal less-than-100-MB full-build claim here.
Native build output is still relocated by the existing per-repository cache.

## Archived chats

Settings now lists archived conversations in bounded pages with name/project search.
A read-only transcript view uses bounded history replay. Unarchive only clears the
archive marker and publishes a state change. It does not start a provider, change
the ended timestamp, recreate a worktree or alter sibling chats. Resuming a chat
later remains a separate action with the existing checkout restoration rules.

## Operational limits

- The older issue's database-backup leak is already addressed on current main:
  startup prunes recognized migration backups while preserving a validated rollback.
  History snapshots already have a retention policy and byte ceiling.
- This change does not delete the user's unverifiable orphan checkout. Its contents
  cannot be established safely, so it remains a manual recovery decision.
- A signed release driven by a live agent is still required for the release-bundle
  disk measurement. Local unit tests and a browser preview do not establish that.
- GitHub billing and Actions capacity are account operations, not code fixes. CI
  status is reported on the PR rather than assumed from the old issue.
