# fix/opencode-read-only-workers — Test Contract

Plan note: `bridge://p/01a1ddb3-48c8-7593-aa43-d7804992bae5`

## Functional Behavior

### OpenCode read-only workers

- A delegation with `writeMode: read_only` routed to OpenCode starts a worker
  instead of failing with `opencode cannot run a read_only worker`.
- The OpenCode server for that worker runs under the platform sandbox
  (`sandbox-exec` on macOS, Landlock on Linux), exactly like Claude and Codex
  read-only workers. Writes to the workspace stay denied by the OS.
- OpenCode's own writable state lives in the worker's private output folder:
  `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and `TMPDIR` point under
  `<output>/.opencode/`, and `BRIDGE_WORKER_OUTPUT_DIR` is the output folder.
  The user's config (`~/.config/opencode`) stays readable, so custom providers
  and MCP servers still load.
- Stored logins are seeded into the sandbox so the worker can reach its model:
  - 2.x: every row of the host `credential` table is copied into the sandbox
    database after the server creates it, before the session is created.
  - 1.x: the host `auth.json` is copied into the sandbox data folder.
  - An OAuth credential is copied with its refresh token cleared, so a
    sandboxed worker can never rotate (and so invalidate) the user's login.
  - An OAuth credential whose access token has already expired is skipped.
  - API key credentials are copied unchanged.
  - The host database is opened read-only and never written.
  - An extra OpenCode account seeds from that account's data home, not the
    default one.
  - No host store at all is not an error: the worker starts with no logins and
    OpenCode reports the missing provider itself.
- A networked read-only worker gets `GH_TOKEN` from the host `gh`, as Claude
  and Codex workers do.
- A read-only worker skips the shared per-repository build cache.
- The cold start under the sandbox gets a longer readiness deadline than a
  normal launch, because a fresh database runs every OpenCode migration.
- `BRIDGE_READ_ONLY_NETWORK=deny` is the one remaining refusal: OpenCode's own
  loopback server cannot run without network, so the descriptor stops
  advertising `read_only` and the adapter refuses with that exact reason.

### Descriptor and routing

- The OpenCode descriptor and built-in contract advertise `read_only`,
  `workspace_write` and `danger_full_access` (unless runtime network is
  denied, then `read_only` is dropped).
- When a pinned or manual route names a harness that cannot run the requested
  sandbox mode (`PermissionCeiling`), the router substitutes a compatible
  harness (tier route first, then the best routable candidate, then any
  eligible one), records `router.harness_substituted`, and appends the reason
  to the decision explanation. The worker runs on the substitute.
- Only when no installed harness supports the mode does routing fail, with a
  message naming the mode in plain words.
- Substitution never changes the sandbox mode, write scope or approvals.

### Worker launch failure card

- `delegation.rejected` with `launchFailed: true` renders a card titled
  "Worker didn't start".
- The card shows a launch track (Policy, Route, Workspace, Start) with the
  failed step marked, derived from `data.phase`; an unknown phase shows no
  track.
- The reason renders as readable prose (sans, foreground), with harness ids
  and sandbox modes humanized for display (`opencode` -> OpenCode,
  `read_only` -> read-only).
- The orchestrator line has its own icon and states one of: notified and not
  waiting; could not be notified.
- The raw reason sits behind a "Details" disclosure in a mono well with a
  Copy button; copying writes the raw (unhumanized) reason.
- A policy rejection (`launchFailed` absent) uses the same card, titled
  "Delegation request rejected", with no track, and the retry line
  ("Asked the orchestrator to fix and resend it" / "Correction limit
  reached").
- Keyboard: Details and Copy are reachable and labelled; the card keeps
  `role="alert"`.
- Light and dark themes both use tokens only (`designSystem.test.ts` passes).

## Unit Tests

Rust (`bridge-core`):
- `opencode_adapter::tests::sandbox_environment_points_every_writable_home_into_the_output_dir`
- `opencode_adapter::tests::seeding_copies_keys_and_strips_oauth_refresh_tokens`
- `opencode_adapter::tests::seeding_skips_expired_oauth_and_never_writes_the_host_store`
- `opencode_adapter::tests::legacy_auth_json_is_sanitized_into_the_sandbox`
- `opencode_adapter::tests::missing_host_store_seeds_nothing`
- `opencode_adapter::tests::read_only_refuses_only_when_runtime_network_is_denied`
- `adapters::tests::opencode_advertises_read_only_unless_runtime_network_is_denied`
  (replaces `opencode_never_advertises_the_read_only_sandbox_it_refuses_to_start`)
- `builtin_compatibility` contract equality still passes with the new modes.
- `learning_router::tests::pinned_incompatible_harness_is_substituted_with_a_compatible_one`
  (replaces `pinned_incompatible_harness_returns_an_actionable_incompatibility`)
- `learning_router::tests::no_compatible_harness_explains_the_sandbox_mode`
  still passes.

Frontend (`AgentConversation.test.tsx`):
- launch failure renders the new title, the track with the failed step, the
  humanized summary, and the orchestrator line.
- unknown phase renders no track.
- policy rejection renders its own title and retry line.
- Details reveals the raw reason; Copy writes the raw reason to the clipboard.

## Integration / Functional Tests

- `opencode_adapter::tests::live_read_only_worker_answers_inside_the_sandbox`
  (`#[ignore]`, needs a signed-in `opencode` on PATH): starts a real
  read-only OpenCode worker through `start_with_settings` with a real
  `ReadOnlySandbox`, runs one turn asking it to read a file in the workspace,
  and checks the answer arrives, the workspace is unchanged, and an attempted
  shell write into the workspace is denied.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar + vitest + cargo workspace).

## E2E Tests

- N/A as an automated suite. Manual: in the desktop app, ask an orchestrator
  to delegate a read-only research task with `harness: opencode`; the worker
  starts and reports back.

## Manual / cURL Tests

- Sandbox probe (already run while planning, re-run after the change):
  `sandbox-exec -f <profile> -- opencode serve` with the redirected XDG homes
  answers `/api/integration` with the user's connected providers.
- Screenshots of the new card, dark and light, plus the Details-open state,
  from a throwaway Vite entry rendering the real component.
