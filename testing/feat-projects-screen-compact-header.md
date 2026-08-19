# feat/projects-screen-compact-header — Test Contract

Three changes the user asked for after #201 landed: projects become their own
screen instead of a tree in the rail, the duplicated header meta line goes, and the
Agent/Changes/Code/Terminal tab row collapses into a compact icon strip.

## Scope And Stated Assumptions

- **Projects leave the rail entirely.** The rail lists chats; projects get a screen
  reached from the footer beside Marketplace and Settings. The rail keeps its
  `workspaces` prop only because `Group by → Project` needs workspace titles for its
  group labels.
- **The history keeps every top-level chat.** With the tree gone from the rail, a
  project's chats are no longer listed twice, so the flat history stays the single
  place every chat is reachable and `Group by → Project` stays meaningful.
- **The header loses its second line.** It currently renders
  `Orchestrator · Claude · Claude Opus · isolated worktree` — the harness twice,
  because the code calls `harnessLabel` in two places. Title, model and repo context
  fold into the one toolbar row; the duplication does not survive.
- **`Tabs`/`TabsList` stop being used for this row.** The underline tab list is
  replaced by a segmented icon control. `src/components/ui/tabs.tsx` stays as it is —
  other screens use it.
- **Floating window controls move into the toolbar.** Browser, the learning-router
  dialog, fullscreen and End currently sit in the header's right edge and in a
  `fixed` cluster that overlaps it at narrow widths (visible in the user's
  screenshot). They move into the toolbar and its overflow menu. Mission Control and
  the usage widget stay in the fixed cluster.
- **One popover implementation.** `SidebarFilterMenu` grew a portal-on-fixed-coords
  panel in #201; the toolbar needs the same thing. That mechanism moves to
  `src/components/ui/menu-panel.tsx` and both use it, rather than a second copy.

## Functional Behavior

### Projects screen
- Reached from a `Projects` entry in the rail footer, above Marketplace. The entry
  is active-styled while the screen is open, exactly like Marketplace and Settings.
- A page header reading `Projects` with a `New project` action.
- One card per workspace: title, path in mono (truncated from the left so the
  leaf directory stays visible), branch, dirty/clean state with `+additions`
  `−deletions` when dirty, and its chat count.
- A card lists up to 6 of its chats, newest first, each with the same status dot as
  the rail. Above 6 it says `+N more`. Clicking a chat opens it and returns to the
  workspace view.
- Each card offers `New agent`, and `Connect folder` when the workspace has no path.
- With no workspaces the screen shows one empty state and the `New project` action,
  not an empty grid.

### Session toolbar
- One row replaces the old title block and the tab strip: title, segmented tab
  control, quiet context text, overflow menu.
- Tabs are icon-only except the active one, which also shows its label. Every tab
  keeps `title` and `aria-label`, and the active one carries `aria-pressed="true"`.
- `Changes` shows its dirty-file count whether or not it is the active tab.
- The segmented control is absent when the session has no repo — a direct chat has
  one panel, so a one-item control would be furniture.
- Context text reads `<model>` and, for a repo session, `isolated worktree` or
  `<branch> · N changed` / `<branch> · clean`. It hides below `lg`, where the row
  has no room for it; the title and tabs never hide.
- The overflow menu holds Browser (checked while open), Learning router settings
  (repo sessions only), Fullscreen, and End chat (only while the session is live).
  End is styled destructive and disabled while a turn is in flight.
- In fullscreen the row is the topmost strip, so it keeps the left inset that leaves
  the traffic lights their corner and stays a drag region.

### Rail
- No projects tree, no `New project` button in the rail, no per-workspace rows in
  the collapsed rail.
- Footer order: Projects, Marketplace, Settings.
- Everything else from #201 is unchanged: day headers, filter/grouping popover,
  search, the 12-row cap, collapse, resize, the below-`sm` drawer.

## Unit Tests

`src/components/ProjectsScreen.test.tsx` — new:
- Renders one card per workspace with title, branch and chat count.
- Shows `+additions`/`−deletions` when dirty and `clean` when not.
- Lists at most 6 chats and reports `+N more` beyond that.
- Offers `Connect folder` only when the workspace has no path.
- Renders a single empty state, and still the `New project` action, with no
  workspaces.
- Clicking a chat calls `onOpenSession` with its id.

`src/components/SessionToolbar.test.tsx` — new:
- Renders the title once, and no second meta line (no `Orchestrator · Claude ·
  Claude Opus` duplication).
- Active tab shows its label; inactive tabs are icon-only but keep `aria-label`.
- `Changes` renders its count while inactive.
- No segmented control when `tabs` has one entry.
- Overflow menu lists Browser, Fullscreen, and End only when the session is live;
  End is absent otherwise.
- Selecting a tab calls `onTabChange` with its id.

`src/components/BridgeSidebar.test.tsx` — extended:
- Renders no projects tree and no `New project` control.
- Renders a `Projects` footer entry, and marks it active when `projectsActive`.
- Existing cases stay green, including `Group by → Project` labels, which still
  need the `workspaces` prop.

`src/components/SidebarFilterMenu.test.tsx` — unchanged, and must stay green across
the `MenuPanel` extraction. It is the proof the refactor kept behaviour.

## Integration / Functional Tests

- `src/App.test.tsx` green with the new view and the reduced sidebar props.
- `npx tsc -b` — proves no caller still passes `expanded`, `onToggleWorkspace`,
  `onNewWorkspace`, `onNewWorkspaceSession`, `onConnectFolder` or `busy` to the rail,
  and that the `view` union covers `projects` everywhere it is switched on.
- `src/designSystem.test.ts` — unchanged; the new screen and toolbar must introduce
  no palette class, hex literal, white/black wash, or blur on a resting surface.

## Smoke Tests

- `npx vitest run` — green.
- `npx tsc -b --pretty false` — no diagnostics.
- `rg -n "harnessLabel\(session.harness\)" src/App.tsx` — at most one hit, and not
  two in one line.
- `rg -n "TabsTab" src/App.tsx` — no hits for the session tab row.

## E2E Tests

N/A — no browser E2E harness for the Tauri shell. Verified instead by building the
real components against mock data and driving them in a browser, as in #201.

## Manual Tests

1. Open the app: the rail has no projects tree; the footer has Projects.
2. Click Projects: one card per workspace with its branch, dirty count and chats.
3. Click a chat on a card: it opens in the workspace view.
4. In a repo session, the header is one row: title, four icon tabs with the active
   one labelled, `Claude Opus · isolated worktree` at the right.
5. Switch tabs from the icon strip; Changes keeps its count badge while inactive.
6. Open the overflow menu: toggle Browser, enter Fullscreen, End the session.
7. Narrow the window: context text drops out before the tabs do; nothing overlaps
   the Mission Control / usage cluster.
8. `Group by → Project` in the rail still labels groups with project names.
