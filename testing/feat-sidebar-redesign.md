# feat/sidebar-redesign — Test Contract

Redesigns the navigation rail: single-line chat rows under sticky day headers, a
filter/grouping popover, a tightened type scale, the projects tree above the
history, and no live-worker panel.

## Scope And Stated Assumptions

- **The rail is the only surface that changes.** `workerStatus.ts` stays exactly as
  it is — Mission Control and `App.tsx` are its other callers and they keep the
  worker view. What goes away is the rail's copy of it.
- **`SidebarWorkerPanel` is deleted, not hidden.** With the rail no longer rendering
  it, nothing imports it, and a dead component with its own test suite is worse than
  no component. Live worker state remains reachable through Mission Control, which
  is where a grid of workers belongs. This is a deliberate loss of one thing: the
  two-line activity excerpt ("Implemented GFM table + …") is no longer visible
  without leaving the current chat.
- **The history list now includes workspace chats.** Today it renders
  `standaloneChats` only, so a chat inside a project is reachable only by expanding
  that project. Grouping by date is worthless if the busiest sessions are excluded,
  and `Group by → Project` cannot exist without them. A chat therefore appears twice
  when its project is expanded — once in the tree, once under its day — which is how
  Codex and Claude Code both behave. The prop `standaloneChats` + `workspaceChats`
  collapses to a single `chats: Session[]` (all top-level sessions) and the rail
  derives each project's children itself.
- **The branch/dirty line under an expanded project is removed** at the user's
  direction. `Workspace.branch` and `dirtyFiles` are still rendered by the chat
  header in `App.tsx`, so the information is not lost from the app.
- **Day buckets come from `session.startedAt`** (falling back to `endedAt`). Session
  has no last-activity field; inventing one is out of scope. Sessions with neither
  timestamp fall into a trailing `Earlier` bucket rather than silently sorting as
  epoch-0.
- **No new dependency.** The popover is built from a button and a positioned div;
  `@base-ui/react` ships a menu, but the rail needs a two-level drill-in that its
  API does not model cheaply, and the project has no existing menu primitive to
  extend.

## Functional Behavior

### Chat rows
- One line per chat: status dot, then title (`title || label`), truncated.
- Harness and model are **not** in the row's visible text. They are in the row's
  `title` attribute as `<title> — <Harness> · <model>`, and the model alone appears
  on the active row.
- Row is 28px tall with a 6px radius. Active row = `bg-accent` + `text-foreground` +
  medium weight, no card fill, no border, no inset ring.
- Status dot: `working → bg-success`, `waiting → bg-warning`,
  `failed → bg-destructive`, `ready → bg-info`, everything else a faint
  `bg-muted-foreground/25` that holds the row's left alignment without reading as a
  signal.

### Day headers
- Groups are labelled `Today`, `Yesterday`, then `Aug 17` for an earlier day in the
  current year and `Aug 17, 2025` for an earlier year.
- `Yesterday` is computed by decrementing the calendar date, not by subtracting
  86 400 000 ms, so a DST boundary does not relabel it.
- Each header sticks to the top of the scroller while its group is on screen and
  paints `bg-sidebar` so rows do not show through.
- A header carries the group's count on the right in mono.

### Filter / grouping popover
Anchored to an icon button on the `Chats` header row. Root panel rows, each showing
its current value and a chevron:

| Row | Options | Default |
| --- | --- | --- |
| Status | All · Active · Waiting · Failed | All |
| Agent | All, then one entry per harness present in the list | All |
| Group by | Date · Project · Agent · Status · None | Date |
| Sort by | Recency · Name | Recency |

- Clicking a row drills into its options; the options panel has a back control
  naming the row. Choosing an option applies it and returns to the root panel, so
  several settings can be changed in one visit.
- Escape and an outside click close the popover. The trigger reflects open state
  with `aria-expanded`; the current option carries `aria-checked="true"`.
- All four settings persist to `localStorage` under `bridge.sidebar.chatView`. An
  unknown or corrupt stored value falls back to the default for that field rather
  than throwing.

### Status filter buckets
- `Active` = `working` | `starting` | `resuming` | `restored` | `checkpointing` | `warm`
- `Waiting` = `waiting`
- `Failed` = `failed`
- `All` = no filtering

### Grouping
- `Date` — one group per calendar day, newest day first.
- `Project` — one group per workspace that has a matching chat, then a trailing
  `No project` group. Project groups are ordered by their most recent chat under
  `Sort by → Recency`, alphabetically under `Sort by → Name`.
- `Agent` — one group per harness label, largest group first, ties alphabetical.
- `Status` — fixed order: Active, Waiting on you, Failed, Idle.
- `None` — a single unlabelled group; no header is rendered.
- `Sort by → Recency` orders chats newest first inside every group;
  `Sort by → Name` orders them by `title || label` with `localeCompare`.

### Search
- The search icon on the brand row toggles a filter input; toggling it off clears
  the query.
- A query matches case-insensitively against title, label, harness label and model,
  and is trimmed before matching.
- While a query is active, a project with a matching name or a matching child
  renders expanded regardless of the `expanded` set, and a project with no match is
  hidden. The parent's `expanded` state is not mutated by searching.

### Projects
- The projects section renders **above** the chat history.
- The Status and Agent filters narrow the tree exactly as the search query does:
  the rail shows one filtered set of chats, viewed two ways. A project whose chats
  are all filtered out is hidden, and its count reflects what passes the filter.
- A project row shows chevron, folder icon, title, and its chat count.
- Expanding shows its chats, a `New agent` action, and `Connect folder` when the
  workspace has no path. No branch/dirty line.

### Truncation
- A group renders at most 12 rows, then a `Show N more` control that reveals the
  rest of that group. The cap is per group key and resets when the grouping mode
  changes.

### Type scale
At most four styles in the rail: 14px semibold (wordmark), 13px (row titles, new
chat, project titles), 11px (section and group labels, footer, inline actions), 10px
mono (counts, model on the active row). No `uppercase` + `tracking-[0.16em]` label,
no gradient rule, and no 8px/9px/9.5px text anywhere.

### Unchanged
Collapse/expand, persisted width, drag-to-resize, the below-`sm` off-canvas drawer
and its scrim, the marketplace and settings footer entries, and every existing
callback prop other than the worker and chat-list props named above.

## Unit Tests

`src/components/sidebarChats.test.ts` — new, pure module under test:

- `dayLabel` — returns `Today` for now, `Yesterday` for the previous calendar day,
  `Aug 17` for an earlier day this year, `Aug 17, 2025` across a year boundary.
- `dayLabel` — a 23-hour gap across a spring-forward boundary is still `Yesterday`.
- `groupChats` `date` — days descend; chats inside a day descend by `startedAt`.
- `groupChats` `date` — a chat with neither `startedAt` nor `endedAt` lands in a
  trailing `Earlier` group.
- `groupChats` `project` — workspace titles become labels, unassigned chats land in
  a trailing `No project` group, and a workspace with no matching chat is absent.
- `groupChats` `project` — group order follows most-recent chat under `recency` and
  `localeCompare` under `name`.
- `groupChats` `agent` — one group per harness label, largest first, ties
  alphabetical.
- `groupChats` `status` — fixed bucket order, and `warm` lands in `Active`.
- `groupChats` `none` — exactly one group with an empty label.
- `groupChats` — `sort: "name"` orders by `title || label`, falling back to `label`
  when `title` is null.
- `filterChats` — matches title, label, harness label and model,
  case-insensitively; whitespace-only query matches everything.
- `filterChats` — `status: "active"` keeps only the active bucket; `agent: "codex"`
  keeps only that harness; the two compose.
- `agentOptions` — one entry per harness present, deduped, alphabetical.
- `readChatView` — returns defaults with empty storage, round-trips a written
  value, and falls back per field on an unknown value or unparseable JSON.

`src/components/SidebarFilterMenu.test.tsx` — new:

- Root panel lists the four rows with their current values.
- Drilling into a row lists its options with `aria-checked` on the current one.
- The trigger carries `aria-expanded` reflecting open state.

`src/components/BridgeSidebar.test.tsx` — extended, existing cases kept:

- No `Live workers` text, and no worker panel markup, in either the expanded or the
  collapsed rail.
- Day headers `Today` and `Yesterday` render for chats stamped accordingly.
- The projects section precedes the chats section in the rendered markup.
- No branch/dirty line: an expanded workspace with `branch: "main"` and
  `dirtyFiles: 3` renders neither `main ·` nor `3 changed`.
- A row's visible text excludes harness and model while its `title` attribute
  includes both.
- Existing: off-canvas drawer classes, dismiss scrim, resize handle, `bg-sidebar`
  with no literal colours, semantic status tokens.

`src/designSystem.test.ts` — unchanged and must stay green; it is the guard that the
rewrite introduces no palette class, no white/black alpha wash, no hex literal and
no blur on a resting surface.

## Integration / Functional Tests

- `src/App.test.tsx` must stay green: `App` is the only consumer of
  `BridgeSidebarProps` and the prop change (`standaloneChats` + `workspaceChats` →
  `chats`, minus `workers` / `workerRuntimes` / `workerReasons`) has to compile and
  render there.
- `npm run check` — `tsc -b` proves no caller still passes a removed prop and no
  import of the deleted component survives; `cargo check` is unaffected but runs as
  part of the script.

## Smoke Tests

- `npx vitest run src/components src/designSystem.test.ts src/App.test.tsx` — green.
- `npx tsc -b --pretty false` — no diagnostics.
- `rg -n "SidebarWorkerPanel" src` — no hits.
- `rg -n "text-\[(8|9|9\.5)px\]" src/components/BridgeSidebar.tsx` — no hits.

## E2E Tests

N/A — the repo has no browser E2E harness for the Tauri shell. The visual result is
verified against `docs/sidebar-redesign.html`, the mockup the design was approved
from, and by the static-render assertions above.

## Manual Tests

No cURL surface. Manual pass, in the running app:

1. Open the rail with a history of 20+ chats — rows are single-line and day headers
   read `Today` / `Yesterday` / a date.
2. Click the filter icon on the `Chats` header — set `Status → Active`, confirm only
   in-flight chats remain, reopen and confirm the row reads `Active`.
3. Set `Group by → Project`, confirm groups are project names plus `No project`.
4. Reload the app — the four settings survive.
5. Toggle search, type a fragment of a project name — that project renders expanded
   and non-matching projects disappear; clear it and the tree returns to its
   previous expansion.
6. Collapse the rail — icon-only, and no worker tile.
7. Narrow the window below `sm` — the rail becomes the off-canvas drawer, scrim
   dismisses it.
8. Switch light/dark — every surface still resolves from tokens.
