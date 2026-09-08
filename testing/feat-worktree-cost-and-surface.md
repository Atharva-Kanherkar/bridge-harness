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
- Read-only sandboxed workers keep their redirected `HOME`/`TMPDIR`; the cache
  directory is added as an explicitly writable root so the sandbox does not
  refuse it.
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
  the same classification and refusals as the sweep. It never forces.
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
- A chat can be archived from the UI: a per-chat action, and a per-chat control
  in its settings.
- Archiving reclaims the chat's worktrees, because that is what `archive_workspace`
  already does — this slice supplies the missing caller, which is the original
  reason worktrees accumulated at all.
- Before archiving, the user is told what will be reclaimed.
- The existing refusal is inherited, not bypassed: an archive is refused when a
  worker worktree holds uncommitted work, and the refusal names how many.

### CLI
- `bridge exec worktrees list --json` and `worktrees sweep --json` for ops.

## Unit Tests

- `build_cache_env_is_keyed_by_repository` — two worktrees of one repo produce
  the same cache directory; a different repo produces a different one.
- `build_cache_env_is_absent_without_a_repository`
- `build_cache_env_never_overrides_an_operator_setting`
- `a_sandboxed_worker_can_write_to_its_build_cache`
- `a_settled_worker_branch_contained_in_its_adopter_is_deleted`
- `a_worker_branch_with_unique_commits_survives`
- `an_orchestrator_or_pull_request_branch_is_never_deleted`
- `reclaim_refuses_a_checkout_bridge_did_not_create`
- `reclaim_runs_the_same_classification_as_the_sweep`

## Integration / Functional Tests

- `archive_from_the_ui_reclaims_the_worktrees_and_reports_the_bytes`
- `archive_is_refused_when_a_worker_worktree_is_dirty` — existing behavior still
  holds through the new caller.
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
