# feat/dock-browser — test contract

Locked before implementation. One workstream: the Browser Surface becomes a
dock tenant. The supervised browser is the surface where seeing both sides
costs the most — the agent acts on a page, the human approves or takes over
in the conversation — so it moves into the dock beside the chat, keeps its
lease through every UI toggle, stops polling at foreground rate when nobody
is looking, and announces "waiting for you" where it can be seen: on the
pane switcher.

## Shape of the thing

BrowserSurface stops being a self-positioned side panel: its root fills
whatever container hosts it, and the dock provides the frame. The dock pane
union grows a browser pane between terminal and transcript (the transcript
chord moves from four to five), available in every session — a browser
needs no repository. The standalone browserOpen state and its render are
gone; the toolbar menu's Browser item opens the pane. The surface polls its
snapshot at 700ms while visible, drops to 5000ms while hidden with a lease
held, and does not poll at all while hidden without a lease. It reports its
supervision status upward; App turns waiting_for_you or a pending sensitive
approval into an attention mark on the switcher and the collapsed rail —
the dock shell's descriptor grows an alert flag rendered as a pulsing
warning dot. The shell's keep-mounted lifecycle already guarantees the
lease survives pane switches and collapse; the tests assert it for this
pane specifically. The unknown-pane persistence fixture moves to a still
unknown id.

## 1. The docked surface — `src/components/BrowserSurface.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The root fills its host | no self-positioning width classes (flex basis, border-l) on the root; it carries h-full |
| 1.2 | Visible polling runs at foreground cadence | with fake timers, snapshot fetches tick at 700ms while visible |
| 1.3 | Hidden with a lease polls at background cadence | after hiding, the next fetches tick at 5000ms |
| 1.4 | Hidden without a lease does not poll | no further fetches after the initial one while hidden and unattached |
| 1.5 | Supervision status is reported upward | onStatusChange fires with the snapshot status, and reports attention for waiting_for_you and for a pending approval |

## 2. The switcher tells on it — `src/components/SessionDock.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | An alert descriptor marks the tab | the pane's tab renders the pulsing attention dot |
| 2.2 | The collapsed rail carries the same mark | the rail button renders the attention dot |

## 3. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | The toolbar menu opens the browser pane | the Browser menu item selects the browser pane in an open dock |
| 3.2 | The pane exists for direct chats | in a direct chat the browser pane opens with the surface, not an unavailable frame |
| 3.3 | The surface survives a pane switch | the same surface node is in the DOM, hidden, after switching to another pane |
| 3.4 | Waiting for you reaches the switcher | with the snapshot reporting waiting_for_you, the browser tab shows the attention dot while another pane is active |
| 3.5 | The transcript chord moved with the order | the fifth pane chord opens the transcript |

Explicitly **not** changed: the surface's own panes (page, elements,
timeline, debug, metrics) and their behaviour; attachment, lease, takeover,
and approval flows; the prompt-injection and redaction notices; the
detach-on-session-switch guard; the policy engine; the prior dock
contracts except the two named evolutions (pane order, unknown-pane
fixture). The design-system guard stays green with no new allowlist
entries.
