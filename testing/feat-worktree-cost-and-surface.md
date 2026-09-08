# feat/worktree-cost-and-surface — Test Contract

Closes the P2 and P3 halves of #574. Locked before implementation.

PR #577 made worktrees *visible and reclaimable*. It did not make them cheap: a
checkout is ~30 MB until an agent builds in it, after which it is gigabytes. This
change attacks the cost and then puts the whole thing in front of a person.

## Functional Behavior

### Shared build caches (the actual lever)
- Every agent session process and every verification command run in a Bridge
  worktree receives `CARGO_TARGET_DIR` and package-manager cache variables
  pointing at a **per-repository** cache under the data directory, not at a
  directory inside the worktree.
- The cache key is the repository's main worktree, so every checkout of one
  repository shares one cache and a second worker gets warm artifacts.
- A process with no resolvable repository (a scratch chat) gets no cache
  variables rather than a wrong shared one.
- A variable the user has already set in the environment is never overridden.
- Read-only sandboxed workers are **excluded** on purpose. Their seatbelt
  profile permits writes only under their own output directory, and adding a
  shared cache as a writable root would widen the read-only guarantee to buy
  build speed for workers that are not supposed to be building. They keep their
  redirected `HOME`/`TMPDIR` and build into their own output directory if they
  build at all.
- Expected effect, stated as the goal: a fresh worker worktree stays under
  100 MB after a full `bun run check`, against ~3.9 GB today.

### Worker scratch-branch cleanup
- A `<task>-worker-<uuid>` branch is deleted once its worker's binding is
  terminal (`adopted`, `discarded`, `empty`) **and** its tip is contained in the
  branch that adopted it or equals its base commit.
- `bridge/*` orchestrator branches and pull-request branches are never deleted.
- A branch holding commits that exist nowhere else is never deleted, whatever
  its binding says.

### Reclaim on demand
- `worktrees/reclaim_worktree` reclaims one inventoried checkout by id, running
  the same classification as the sweep. It never forces: `at_risk`, `retained`
  and `unverifiable` are refused with their reason, and the refusal is what the
  caller receives rather than an error.
- One deliberate difference from the sweep: an explicit request *does* reclaim a
  `pushed_unmerged` checkout, which the sweep retains by default. The
  distinction is who is asking. The sweep runs unattended and defaults to
  caution; a person clicking Reclaim on a row that says "every commit is on a
  remote" has been told exactly what they are discarding. Nothing unique to the
  disk is ever removed on either path.
- `worktrees/sweep_worktrees` runs a full maintenance pass and returns its
  outcome.
- Reclaiming a checkout Bridge did not create is refused, naming why.

### Storage surface
- Settings gains a Storage section showing total worktree bytes against the cap,
  a per-repository rollup, and a per-checkout table: kind, branch, state,
  disposition, age, size, and the retained reason in words.
- Each row a person may act on offers Reclaim; rows nothing may touch show why
  instead of an inert button.
- A Sweep action runs the pass and reports what was freed.
- External checkouts are listed read-only and visibly not Bridge's to reclaim.

### Archiving a chat (closing slice)

**Contract corrected during implementation.** The locked version had this slice
supply "the missing caller" for `archive_workspace`. That would have been
destructive: `archive_workspace` deletes *every session in the workspace*, and a
workspace holds many chats — on the machine this was written for, one holds 836
root sessions and only two of eight hold a single chat. A per-chat button wired
to it would have destroyed hundreds of unrelated conversations to reclaim one
directory. Archiving a chat is therefore its own operation.

- A chat can be archived from the UI, per chat, from its row in the sidebar.
- Archiving reclaims **the checkout that chat owns** — its own orchestrator
  worktree, which the inventory tracks per session — and never the workspace's,
  which belongs to every other chat in that project.
- History is kept. The session row, its forest entries and its evidence all
  survive; the chat is marked archived and stops being listed.
- The worktree goes through the same classification and refusals as any other
  reclaim, and a checkout that cannot be proven expendable is **kept rather than
  blocking the archive**: filing a conversation away should not require first
  resolving its uncommitted work. The reason is reported to the caller.
- A chat that is still running is refused until it is stopped.
- Before archiving, the user is told that history is kept and the worktree is
  not.

### CLI
- `bridge exec worktrees list --json` and `worktrees sweep --json` for ops.

## Unit Tests

- `build_cache_env_is_keyed_by_repository` — two worktrees of one repo produce
  the same cache directory; a different repo produces a different one.
- `build_cache_env_is_absent_without_a_repository`
- `build_cache_env_never_overrides_an_operator_setting`
- `a_read_only_sandboxed_worker_gets_no_shared_cache`
- `a_settled_worker_branch_contained_in_its_adopter_is_deleted`
- `a_worker_branch_with_unique_commits_survives`
- `an_orchestrator_or_pull_request_branch_is_never_deleted`
- `reclaim_refuses_a_checkout_bridge_did_not_create`
- `reclaim_runs_the_same_classification_as_the_sweep`

## Integration / Functional Tests

- `archiving_a_chat_reclaims_its_own_worktree_and_leaves_its_siblings_alone`
- `an_archived_chat_keeps_its_row_and_stops_being_listed`
- `archiving_keeps_a_dirty_checkout_and_says_so_instead_of_refusing`
- `archiving_refuses_a_chat_that_is_still_running`
- Protocol: registry, typed payloads, dispatch, Tauri command and generated
  artifacts stay 1:1 — enforced by the existing drift gates.

## Smoke Tests

- Frontend mock mode renders the Storage section from `api.ts` mocks with no
  Tauri host.
- `bun run dev` boots and Settings → Storage renders without a runtime error.

## E2E Tests

N/A — the repository has no browser E2E harness. The `run` skill's release-bundle
path is the manual equivalent, covered under Manual below.

## Manual Tests

1. Build a release bundle, open a chat with an isolated worktree, ask the agent
   to run `bun run check`, then confirm `src-tauri/target` inside the worktree
   stays empty and the shared cache under the data directory grows instead.
2. Settings → Storage: confirm totals, at least one reclaimable row, one
   retained row showing its reason, and that Reclaim frees the space.
3. Archive that chat from its own menu; confirm the worktree is gone and the
   reported byte count matches.
4. `bridge exec worktrees list --json | jq '.[0]'`.

## Gates

`bun run check`, `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
(plus `--no-run` after any signature change), `bridge-protocol` drift,
`bunx vitest run`, sidecar `node --test`.
