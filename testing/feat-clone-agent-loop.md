# feat/clone-agent-loop — test contract

The full lifecycle the scenario is actually about: **the agent asks Bridge for a
browser → Bridge asks the person → the person allows → Bridge builds it → the
agent acts → Bridge destroys it**, any number of times, concurrently across
chats. Stacked on the trigger (#760) and everything before it. Part of #758.

## Shape of the thing

- The agent is offered an **"ask for a clone" capability every turn** (before any
  clone exists): `live_turn` injects `orchestrator.request_capability_context`.
  The agent calls it with a domain; that records a pending request — it does not
  build anything.
- The dock turns a pending request into an **Allow/Deny card**. `clone_state`
  now returns a `requested` snapshot carrying `pendingRequest` even with no clone.
- **Allow** (`clones/resolve_clone_request`) builds the clone for that domain,
  imports the sign-in, arms the guard, mints the drive tool, and **approves page
  actions** for the session. **Deny** drops the request.
- Only after approval can the agent **act**: mutating kinds (click/type/scroll/
  navigate/focus) are refused until the session is approved; reads
  (inspect/screenshot) always work.
- Destroy / session end / TTL tears the clone down and revokes both capabilities.
- State is keyed per session, so **chats run independent clones at once**.

## 1. The whole loop, real browser — `clone_orchestrator::tests::live` (env-gated)

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The agent asks and the person approves and the agent acts | the agent's `request` over its socket returns 200 and records a pending request; no clone exists yet; `approve_request` builds the clone; a mutating command (scroll) that would be refused now returns 200; destroy tears it down |
| 1.2 | The agent cannot act before approval | with a clone but no approval, a mutating command is refused `403`; a read (`screenshot`) still returns `200` |
| 1.3 | Two chats run independent clones at once | two sessions get different clones; destroying one leaves the other running and reachable |

## 2. Wire + frontend

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | The request surfaces without a clone | `clone_state` returns a `requested` snapshot with `pendingRequest` set when the agent has asked but nothing is approved |
| 2.2 | `resolve_clone_request` is registered and mirrored | protocol registry, params-naming, artifact-drift, and `bridge-deck` command-signature gates pass |
| 2.3 | The dock shows the card | `CloneSurface` renders "The agent wants a signed-in browser" with the domain and Allow/Deny when `status === "requested"`, and the pane pulses attention |
| 2.4 | Allow resolves the request | pressing Allow calls `resolveCloneRequest(session, true)` |

## 3. Repo gates

`bun run check`, `bun run test`, `bun run build` green. The four `updater.test.ts`
and four `bridge-core` PTY failures pre-exist on `origin/main` here.

## What this completes

The end-to-end lifecycle now runs: an agent in a session can ask for a
signed-in browser, the person approves in the dock, the agent drives it
(including clicking and typing), and it is destroyed — repeatable and
concurrent across chats. Proven against a real browser by the live tests above.

## Still ahead (small)

- Streaming frames (`Page.startScreencast`) + input forwarding during takeover;
  today the dock renders a captured frame each poll.
- A TTL scheduler tick calling `sweep_expired` (the method exists).
- The packaged-app screen recording (needs a release bundle).
- Safari import + WebKit (phase 2).
