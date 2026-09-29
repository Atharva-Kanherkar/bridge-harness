# feat/clone-integration — test contract

The slice that was missing: the composition that makes the browser-clone
scenario from #749 actually run. Stacked on parts 1–4 (#754–#757). Issue #758.
macOS only. Out of scope: the approval card, the protocol wire method, and the
frontend's real calls — the last hop that makes it *clickable*; and Safari.

## Shape of the thing

`clone_orchestrator.rs` ties the four parts into one flow: `request_clone`
spawns a guarded clone (part 1), signs it in by importing the approved domain's
cookies (part 3) or by sign-in-inside, arms the guard's allow list and secrets
(part 2), mints the agent tool bound to the agent's runtime process (part 3),
and starts the lease. It also loads the code under test, hands back live frames,
and destroys everything on lease expiry or session end.

Supporting hooks in `browser_clone.rs`: a `--enable-unsafe-extension-debugging`
launch flag (so `Extensions.loadUnpacked` works), a `guarded` constructor, and a
lazily-attached **page target session** (`page_call`) — screenshot, input,
accessibility, and navigate need a page session; the browser endpoint cannot
serve them. Runtime wiring: `BridgeCore` holds the orchestrator (its supervisor
shares the boot ledger, so crash recovery covers its clones), and `live_turn`
injects the orchestrator's capability into any turn whose session has a clone
and destroys the clone when the session clears or stops.

## 1. Composition — `clone_orchestrator::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The approved domain is normalized and validated | `.YouTube.com ` → `youtube.com`; a space or empty string is rejected |
| 1.2 | A view never carries a value | the serialized `CloneView` has status + `minutes_left`, no cookie value |

## 2. The whole scenario, real browser — `clone_orchestrator::tests::live` (env-gated)

`the_whole_scenario_runs_against_a_real_browser`, under `BRIDGE_CLONE_LIVE=1`,
run against Chrome and Brave. One test walks the entire path:

| # | Step | Assertion |
|---|---|---|
| 2.1 | Request a clone | `request_clone` returns a guarded clone signed in for the domain |
| 2.2 | The agent tool is bound to the agent | a capability is minted for the session's runtime process |
| 2.3 | The code under test loads | an unpacked extension loads into the clone and `loadUnpacked` returns an id |
| 2.4 | The agent drives it over its real socket | a `screenshot` command returns `200 {"ok":true}` through the unix-socket tool; an `eval` command is refused `403` |
| 2.5 | The person can see it | `frame` returns a non-empty PNG |
| 2.6 | The lease is the boundary | after the TTL, `sweep_expired` destroys the clone; it is gone (no view, unreachable) |

This is the end-to-end proof the four parts compose. It exercises the guarded
spawn, the page-session attach, the agent tool over its socket with the real
capability check, extension loading, the frame path, and lease teardown, in one
run on two real browsers.

## 3. Runtime wiring — compiled, `bun run check`

- `BridgeCore.browser_clone_orchestrator` is constructed in both the test and
  boot paths; the guarded supervisor shares the boot ledger.
- `live_turn` merges the orchestrator's `capability_context` into the agent
  turn's application context next to the credential and attached-tab contexts,
  and calls `destroy(session_id)` at the three session-clear/stop sites.
- Every reference is `#[cfg(target_os = "macos")]`; the crate still checks on
  non-macOS.

## 4. Repo gates

- `bun run check`, `cargo test -p bridge-core --lib`, `bun run build` green. The
  four `terminal_workspace`/`api` PTY failures pre-exist on `origin/main` here.
- Parts 1–4 suites unchanged and still pass (55 clone + 4 tool + signin + UI).

## What this still does not do (the honest remainder — #758)

- **No approval-triggered start.** Nothing calls `request_clone` from a click
  yet: that needs the approval card and a protocol wire method. Until then a
  clone is created only by the end-to-end test, not by an agent asking in the
  app. `live_turn` injection is therefore correct but inert until the trigger
  exists.
- **Frames are captured, not streamed.** `frame` returns a fresh screenshot;
  `Page.startScreencast` streaming and input forwarding during takeover are the
  follow-up.
- **The dock still reads the mock** until the wire method replaces it.
