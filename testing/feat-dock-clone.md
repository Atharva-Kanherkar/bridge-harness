# feat/dock-clone: test contract

The dock surface for a throwaway browser clone (browser clones, part 4). The
agent drives a private copy of the browser; the person watches it in the dock,
takes it over for a login or 2FA, and destroys it when done. This slice is the
UI and its mock only. No wire method exposes clones yet, so nothing here
reaches Tauri, and the protocol method and live-turn capability injection are
a separate follow-up. **End-to-end reachability from a clean install is
deferred, not delivered:** in a desktop build the pane opens on an honest "No
clone running" state and the Clones settings say the runtime is not connected.

## Shape of the thing

`CloneSurface` mirrors `BrowserSurface`: its root fills whatever hosts it, it
polls a snapshot at 700ms while visible, 5000ms while hidden with a live
clone, and not at all while hidden without one, and it reports supervision
(status plus an attention flag) upward. Attention is `waiting_for_you` or a
pending approval; taking the clone over clears it, because the person is then
the one acting. The dock pane union grows a `clone` pane, appended last so no
existing ⌥⌘N chord moves (it is ⌥⌘9), available in every session. It rides the
shell's existing keep-mounted lifecycle and the existing pulsing-dot `alert`
descriptor flag, and the toolbar's overflow menu opens it. App draws the dot
only once the pane has been visited, because an unmounted surface is not
polling and cannot vouch for a state. A Clones settings page holds the default
sign-in path (`import` | `sign_in_inside`) and the TTL (default 30 minutes),
read and written through `api.ts`.

Types are UI-local in `src/types.ts` (no generated clone type exists). The
api entries are mock only: outside Tauri they serve an in-memory fixture; inside
Tauri reads report "no clone" and actions refuse.

## 1. The surface: `src/components/CloneSurface.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The root fills its host | h-full, no flex basis, no border-l |
| 1.2 | A running clone shows live view, Take over, Destroy | the redacted frame, both buttons |
| 1.3 | No clone is stated, not faked | "No clone running", no controls |
| 1.4 | Visible polling runs at foreground cadence | fetches tick at 700ms |
| 1.5 | Hidden with a live clone polls at background cadence | next fetch at 5000ms, not before |
| 1.6 | Hidden without a clone does not poll | no fetch after hiding |
| 1.7 | Supervision is reported upward | attention for waiting_for_you and for a pending approval |
| 1.8 | Waiting names its reason | the notice carries the reason; Take over is offered |
| 1.9 | Take over, hand back, destroy transition | in control (attention cleared) → acting → destroyed; destroy asks first, Cancel backs out |
| 1.10 | A destroyed clone stops the hot poll | no fetch while hidden |
| 1.11 | A pending approval blocks the surface | Approve once resolves it and lifts the overlay |
| 1.12 | A failed action is reported | onError receives the message; the clone is unchanged |

## 2. The switcher tells on it: `src/components/SessionDock.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | An alert on the clone descriptor marks its tab | the pulsing dot renders, and clears when the alert does |
| 2.2 | The clone body is keep-mounted | the same node, hidden, after switching to another pane |

## 3. App wiring: `src/App.test.tsx`, `src/components/SessionToolbar.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | The toolbar menu opens the clone pane | the Clone item selects the clone tab in an open dock |
| 3.2 | The surface survives a pane switch | the same viewport node, hidden, then visible again on the ninth chord |
| 3.3 | The pane exists for direct chats | the surface renders, not an unavailable frame |
| 3.4 | Waiting for you reaches the switcher | the mock clone waits; the Clone tab and the overflow control show the dot while another pane is active, and not before the pane was opened |
| 3.5 | The overflow menu lists, opens and marks the pane | item present, alert dot on it, click opens `clone` |

## 4. Settings: `src/components/settings/ClonesPage.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | Defaults | import and 30 minutes, read through the api |
| 4.2 | Read and write round-trip | both changes reach the mock store (reader sees them) and survive a cold remount |
| 4.3 | An off-list stored TTL is kept | 45 minutes is shown, not dropped |
| 4.4 | A failed write is reported | onError fires; the stored value stays displayed |
| 4.5 | No backend is stated | "not connected to the runtime", both controls disabled |

Explicitly **not** changed: `BrowserSurface`, the attached-tab bridge, the
Browser pane and its chord, the policy engine, the generated protocol files,
and the prior dock contracts (a ninth pane appends; no chord moves). The
design-system guard stays green with no new allowlist entries; no new `.css`,
no inline style.

Out of scope, and named so they are not mistaken for done: a real
`browser/clone_*` protocol method, injecting the clone capability into a live
turn, per-session scoping of clone state, pointer input into the clone during
takeover, and the cookie import and sign-in flows themselves.
