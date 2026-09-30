# feat/clone-takeover — test contract

Takeover, input forwarding, and the enforced lease. Stacked on the agent loop
(#764). Part of #758. With this, the person can step in — sign in, finish 2FA —
inside a clone while the agent is paused, and every clone destroys itself when
its time runs out.

## Shape of the thing

- **Takeover pauses the agent.** `take_over` sets the clone `taken_over` and
  revokes the agent's page actions; `hand_back` restores them only if the
  person had approved them. So while the person holds the clone, the agent
  cannot act.
- **The person's input is forwarded.** `forward_input` (wire `clones/clone_input`)
  sends a click, scroll, typed text, or a login key (Enter, Tab, arrows…) to the
  page. Coordinates are a fraction of the viewport, so the dock's scaled frame
  maps onto the page. Refused unless the clone is taken over, so it can never be
  a side door for the agent.
- **The lease is enforced.** A background tick calls `sweep_expired` every few
  seconds; a clone past its deadline is destroyed. The thread holds a `Weak`, so
  it ends with the core.
- **The dock forwards input.** While `taken_over`, a click on the live frame is
  sent to the page, and a "type into the page" field sends text + Enter — for a
  login or a 2FA code.

## 1. The whole thing, real browser — `clone_orchestrator::tests::live`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Takeover pauses the agent; the person types; hand back restores | after the request→approve flow, a mutating agent command works; on `take_over` it is refused `403`; the person clicks and types "hunter2" into the page and the input reads it back; on `hand_back` the agent's command works again |
| 1.2 | The lease is swept | a clone with a 1 ms lease is gone after `sweep_expired` |

(Plus the earlier loop tests unchanged: the agent asks/approves/acts, the pre-approval refusal, and two concurrent independent clones.)

## 2. Wire + frontend

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | `clone_input` is registered and mirrored | protocol registry, params-naming, artifact-drift, and `bridge-deck` command-signature gates pass |
| 2.2 | The frame forwards clicks while taken over | a click on the live view calls `cloneInput(session, {kind:"click", x, y})` with viewport fractions |
| 2.3 | Clicks are inert otherwise | a click while `acting` forwards nothing |

## 3. Repo gates

`bun run check`, `bun run test`, `bun run build` green. The four `updater.test.ts`
and four `bridge-core` PTY failures pre-exist on `origin/main` here.

## What this leaves

- Streaming frames (`Page.startScreencast`) instead of a captured frame per poll,
  for a smoother live view.
- The packaged-app screen recording (needs a release bundle).
- Safari import + WebKit (phase 2).
