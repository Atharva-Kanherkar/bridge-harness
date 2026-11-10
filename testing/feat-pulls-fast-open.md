# feat/pulls-fast-open test contract

Pulls surface polish: rename "Gitplace" to "Pulls", a structured loading state, and
faster surface loads (backend repository-resolution cache + frontend stale-while-revalidate).

## Functional Behavior

Rename (behavior-preserving):
- Sidebar bottom-rail label, chrome title, empty-state copy and every code identifier
  say "Pulls" instead of "Gitplace".
- The `AppView` id is `"pulls"`; `"gitplace"` no longer exists as a view id.
- The remembered-repo localStorage key keeps its stored value
  (`bridge.gitplace.workspace`) so existing users do not lose their choice; only the
  constant's name changes.
- Review-comment jump semantics (`gitplaceJumpStep` → `pullsJumpStep`) are unchanged.

Loading state:
- While `githubStatus` is in flight, the pane shows the normal pane structure
  (header row + list skeleton) rather than a lone centered spinner on an empty screen.
- A short "Connecting to GitHub…" notice rides inside the skeleton; it never renders
  alone on a blank surface.
- Error / not-installed / not-authenticated / no-repository notices are unchanged.

Faster loads:
- Backend: repository resolution is cached per workspace path for the surface cache
  TTL, so `github_status`, `list_prs`, `list_issues`, `repository_overview` and detail
  reads stop re-spawning `git`/`gh` resolution subprocesses on every call.
  Explicit refresh, `connect_repository`, and failed resolutions bypass or drop the cache.
- Frontend: the last-known status/PRs/issues/overview per workspace survive unmount.
  Reopening the surface paints cached content immediately and revalidates in the
  background; a fresh workspace behaves as before.

## Unit Tests

- `navigationHistory.test.ts`: the `AppView` set contains `pulls`, not `gitplace`.
- `pullsJump.test.ts` (renamed from `gitplaceJump.test.ts`): unchanged assertions.
- `PullsScreen.test.tsx` (renamed): unchanged semantics, new names.
- `BridgeSidebar.test.tsx`: the rail button is labelled "Pulls".
- `App.test.tsx`: "Gitplace" button references become "Pulls".
- `github_surface.rs` tests:
  - `repository_resolution_is_cached_within_the_ttl`: warm `resolve_repository`
    performs zero extra resolution subprocess invocations (fake invocation count).
  - `explicit_repository_refresh_bypasses_the_ttl_cache` still passes and extends to
    repository resolution (a refreshed status re-resolves).
  - `connect_repository` clears the resolution cache (stale negative guarded).
  - A failed resolution never enters the cache.
- `GitHubPane.test.tsx`:
  - with a remembered surface, mount paints cached PRs immediately and revalidates;
  - without one, mount shows the skeleton loading state, not a lone spinner.

## Integration / Functional Tests

- `bun run build` (tsc strict + vite): rename leaves no dangling identifiers.
- `cargo test -p bridge-core github_surface`: cache invalidation rules hold.
- `bun run test`: full suite green.

## Smoke Tests

- `bun run build` exits 0.
- `cargo check` exits 0.

## E2E Tests

- N/A for CI: the surface needs a live, authenticated `gh`. Covered by the manual
  eval below, which runs against this machine's real repository.

## Manual / cURL Tests (evals)

Timing eval, run on this machine against the real checkout and real `gh`:

```bash
cargo test -p bridge-core --test github_load_eval -- --ignored --nocapture
```

- Reports cold and warm latency of status + PR-list loads (backend view).
- The same file, copied uncommitted into an `origin/main` worktree, provides the
  baseline numbers quoted in the PR description.
- Frontend reopen latency is demonstrated by the SWR unit test (cached mount paints
  synchronously before any fetch resolves).
