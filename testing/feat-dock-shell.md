# feat/dock-shell — test contract

Locked before implementation. One workstream: the right-hand dock shell — a
trailing pane beside the conversation that hosts the existing Changes, Code,
and Terminal panels without ever covering the chat. This contract covers the
layout state module, the presentational shell, the App integration that
retires the single-slot `activeTab` model, and keyboard reachability. Pane
*contents* are out of scope: the three tenants land unchanged, and the
Browser overlay keeps today's behaviour until its own pane exists.

## Shape of the thing

One dock, trailing edge, per-workspace state (direct chats key by session id).
`src/dockLayout.ts` owns the pure state: a reducer over
`{ open, width, pane, expanded, visited }`, width clamped to
[320, min(760, available − 440)], persisted per key as
`bridge.dock.v1.<key>` — `open`, `width`, `pane`, `expanded` survive a
restart; `visited` is runtime memory for lazy mounting. Collapsed is the
default and a real state: a thin icon rail, never an invisible edge. Expand
promotes the active pane to the full window. Below 760px of section width an
open dock renders as an overlay sheet over the conversation instead of
starving it. Fullscreen (⌥⌘F) hides the dock without unmounting or mutating
it. The `"events"` member of the old tab union is removed along with the
unrendered `EventPanel`.

## 1. Layout state — `src/dockLayout.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The default state is collapsed with the documented defaults | `open` false, `width` 440, `pane` "changes", `expanded` false, `visited` empty |
| 1.2 | `toggle` opens a collapsed dock and collapses an open one | `open` flips across two dispatches |
| 1.3 | Collapsing resets expand | `toggle` on `{open, expanded}` yields `expanded` false |
| 1.4 | `open-pane` activates the pane and opens the dock in one action | from collapsed: `open` true, `pane` set |
| 1.5 | `open-pane` records a visit exactly once | dispatching the same pane twice leaves one entry in `visited` |
| 1.6 | Visits accumulate across pane switches | after visiting changes → code → changes, `visited` is exactly ["changes", "code"] |
| 1.7 | `set-width` clamps to the dock minimum | requested 100 at available 1200 → 320 |
| 1.8 | `set-width` clamps to the dock maximum | requested 2000 at available 4000 → 760 |
| 1.9 | `set-width` never starves the conversation | requested 700 at available 1000 → 560 (available − 440) |
| 1.10 | A viewport too narrow for both minimums pins the dock minimum | any request at available 600 → 320 |
| 1.11 | `toggle-expanded` flips only while open | no-op from collapsed; flips from open |
| 1.12 | The reducer never mutates its input | dispatching returns a new object; the argument deep-equals its snapshot |

## 2. Persistence — `src/dockLayout.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Write-then-read round-trips the durable fields | `open`, `width`, `pane`, `expanded` equal after `writeDockState` → `readDockState` |
| 2.2 | `visited` is not persisted | reading a state stored as collapsed yields empty `visited` |
| 2.3 | Restoring an open dock seeds the active pane as visited | stored `{open: true, pane: "code"}` reads back with `visited` ["code"] |
| 2.4 | Corrupt JSON falls back to defaults | garbage in storage → `defaultDockState()` |
| 2.5 | Wrong field types fall back to defaults | `{open: "yes"}` → defaults |
| 2.6 | An unknown pane id falls back to the default pane | stored pane "browser" reads back as "changes" |
| 2.7 | An out-of-range width is clamped on read | stored 5000 → 760; stored 10 → 320 |
| 2.8 | Keys are isolated | writes under two keys read back independently |
| 2.9 | A throwing storage never breaks the caller | `getItem`/`setItem` that throw → defaults returned, write swallowed |

## 3. The shell — `src/components/SessionDock.test.tsx`

Component tests mount through `react-dom/client` + `act` with a jsdom
docblock, per house convention. The dock is presentational: state and pane
descriptors in via props, one action callback out.

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | An open dock renders the switcher as a tablist and the active pane body | `[role="tablist"][aria-label="Dock panes"]`; active tab `aria-selected` |
| 3.2 | Switching panes hides, never unmounts, a visited pane | after switching away, the previous pane's body is still in the DOM, hidden |
| 3.3 | An unvisited pane is not mounted | only visited panes have bodies in the DOM |
| 3.4 | Collapsed renders the icon rail with every pane one click away | one button per pane; clicking dispatches `open-pane` |
| 3.5 | Collapsing keeps visited pane bodies mounted | rail visible and the previously visited body still in the DOM, hidden |
| 3.6 | The divider is a real affordance | `[role="separator"][aria-orientation="vertical"]`, focusable; ArrowLeft/ArrowRight dispatch `set-width`; double-click dispatches the default width |
| 3.7 | Expand swaps to the full-window layout and back without remounting | same body node identity before, during, and after |
| 3.8 | An unavailable pane explains itself instead of rendering empty | unavailable descriptor → reason text and the connect-folder action, no pane body |
| 3.9 | Badges ride the switcher and the rail | a count badge shows its number on the tab; the rail shows a dot for the same pane |
| 3.10 | The dirty count is rendered whether or not Changes is active | badge present on the inactive tab |

## 4. App integration — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The conversation and an open pane coexist | with the dock open, the conversation region and the pane body are both in the DOM |
| 4.2 | The composer stays usable while a pane is open | textarea present and enabled alongside an open dock |
| 4.3 | The old tab model is gone | no `activeTab`/`visitedTabs` state; `"events"` absent from the codebase; `EventPanel` deleted |
| 4.4 | The toolbar trades the tablist for a dock toggle | no `[role="tablist"]` in `SessionToolbar`; a toggle with `aria-pressed` reflecting dock state |
| 4.5 | Dock state is keyed per workspace | switching workspaces swaps to that workspace's persisted state; switching back restores the first |
| 4.6 | Pane hosts stay keyed by workspace | tenant containers carry `key={workspace.id}` so buffers and paths never survive a change of tree |
| 4.7 | A direct chat dims repo panes and explains | all three panes render disabled-with-reason; opening one shows the explanation frame |
| 4.8 | Fullscreen hides the dock without destroying it | ⌥⌘F removes the dock from view; exiting restores the same state and mounted bodies |
| 4.9 | Narrow sections render the open dock as a sheet | below the 760px threshold: scrim present, clicking it collapses; no divider |

## 5. Keyboard — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | ⌥⌘0 toggles the dock | open state flips; ⌥⌘F stays fullscreen and is untouched |
| 5.2 | ⌥⌘1–3 open panes by switcher order | pane activates and dock opens, including from collapsed |
| 5.3 | ⌥⌘↩ toggles expand while open and is inert while collapsed | expanded flips only when open |
| 5.4 | Escape restores an expanded dock before it exits fullscreen | one Escape: expanded → false, fullscreen untouched; a second exits fullscreen |
| 5.5 | Every keyboard action has a pointer twin | toggle button, tabs, rail, and expand control cover the same transitions |

Explicitly **not** changed: `BrowserSurface` and its `browserOpen` overlay
toggle; `ChangesPanel`, `CodePanel`, and `TerminalPane` internals and props;
the conversation renderer; the policy engine; every `u-*` surface class. The
design-system guard (`src/designSystem.test.ts`) must stay green with no new
allowlist entries.
