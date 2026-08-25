# feat/right-rail-composer — Test Contract

Ship the real left rail (windowed layout), drop the Work/Code scope switch,
open Automations (not the catalog) from the rail, and start New Chat in the
current/last repo with a composer context strip.

This contract **supersedes**, by name:

- `testing/feat-real-cursor-rail.md`: rail side, `pl-24` on the rail strip,
  scope switch and Needs-you placement, "Chats" vs "Repositories" labels,
  Automations → Marketplace, New Chat → `NewChatDialog`.
- `testing/feat-sidebar-redesign.md` and
  `testing/feat-projects-screen-compact-header.md`: scope persistence
  (`bridge.sidebar.scope`) and the Needs-you rail row.

It **keeps**: hidden session kinds, `CHAT_VIEW_KEY` filter persistence,
`GROUP_ROW_CAP = 12` with search bypassing the cap, git badge from
`Workspace.branch`, `chatListTime`, collapse / persisted width / drag-to-resize
/ below-`sm` drawer.

Plan: `docs/plans/right-rail-and-composer.md`. PRs target
`feat/cursor-sidebar-dev`, never `main`.

## Scope And Stated Assumptions

- Cloud and SSH hosts are disabled menu rows.
- Branches are real local Git refs. Listing is read-only; checkout refuses a
  dirty workspace, a running or history-bearing session, remote-only refs, and
  isolated worktrees. An empty idle session may switch branches.
- `WorkView` is a rail destination labeled **Work board** (not the old Work/Code
  scope switch, and not a Needs-you count badge).
- `NewChatDialog` is deleted. Projects "+" still uses `OrchestratorCreateDialog`.
- The look-at preview (`?preview=right-rail`) is lazy-loaded from `main.tsx`;
  `App.tsx` never imports it.

Locked §10 defaults: one host menu; worktree toggle chip; no "Run in Cloud"
shortcut; Needs-you count off the rail; "Browse catalog" on Automations;
fullscreen is a flush rectangle.

## Functional Behavior

### Shell (left rail)

- Windowed: aside then canvas. Aside `left-0`, `border-r`, drawer closed state
  `-translate-x-full`. Panel icon `PanelLeft`.
- Expanded windowed chrome strip: `h-11`, `u-traffic-inset pl-24`, panel then
  chevrons (`ml-auto`), `data-tauri-drag-region="deep"`.
- Collapsed windowed chrome: the same traffic-light inset, panel only, no
  chevrons. The expand button must not sit under the native traffic lights.
- `showWindowNav={false}`: rail does not render panel or chevrons.
- Resize handle on the rail's **right** edge.
- Labels stay "Hide sidebar" / "Show sidebar".
- Fullscreen chrome is flush (`data-fullscreen` / `data-flush-window`); the
  in-app fullscreen control toggles only the layout flag, not native zoom.

### List

- No `ScopeSwitch`, no `ChatScope`, no `bridge.sidebar.scope` reads/writes.
- No Needs-you count badge. Work opens from the **Work board** action row
  (`onOpenWorkBoard`), not from a Work/Code switch.
- Section label is always `Repositories`.
- `DEFAULT_CHAT_VIEW.groupBy` is `"project"`. A persisted `groupBy` is honored.
- `allowProjectGrouping` is always true.
- Empty copy: one sentence, no "New chat asks which project".
- Plain and project chats appear in the same list (plain under "No project").

### Action rows

| Label | Handler |
| --- | --- |
| New Chat | `onOpenNewChat` — not a modal |
| Search | toggles the filter input |
| Automations | `onOpenAutomations` |
| Mission Control | opens the agent grid |
| Projects | `onOpenProjects` |
| Memory | `onOpenMemory` |
| Work board | `onOpenWorkBoard` |

- Automations is current when `automationsActive`.
- Projects and Memory sit directly below Mission Control.
- The footer is a local-user row. Its username and gear open Settings.

### Automations view

- `AppView` includes `"automations"`.
- Rail Automations → `AutomationsPanel` only. No Agents/Plugins/Skills
  segmented control on that screen.
- Header offers a "Browse catalog" control that opens `MarketplaceScreen`.

### New Chat + composer strip

- Rail New Chat does not render `NewChatDialog` or the string
  "Where should it run?".
- Workspace resolution, first match: active session `workspaceId` (if it still
  exists) → `localStorage["bridge.chat.lastWorkspaceId"]` (validated) → the
  only workspace → else today's direct `createChat` path.
- Repo path: `createWorkspaceSession(id, false)` then focus the created
  session. Persist last workspace id on open/create of a workspace session.
- `ComposerContextStrip` above the dock composer (and on Welcome when a repo
  resolves): repository menu, local branch menu, worktree toggle, host menu.
- Host: This Mac selected. Cloud and SSH are **disabled** with hint copy.
- Repo menu and worktree toggle are enabled only before the first user turn.
  Branch switching follows the same lock and is unavailable in isolated
  worktree mode. After that the worktree and branch chips are static labels.
  Retargeting repo on an empty chat creates a fresh session and stops the empty
  one.
- No "Run in Cloud" pill.

## Unit Tests

`src/lastWorkspace.test.ts` — resolution order and stale-id fallthrough.

`src/components/ComposerContextStrip.test.tsx` — chips render; Cloud/SSH
disabled; worktree/repo/branch locked after first turn; a local branch selection
calls the checkout handler.

`src/components/sidebarChats.test.ts` — drop `ChatScope` / `inScope` cases;
`DEFAULT_CHAT_VIEW.groupBy === "project"`; unknown stored `groupBy` falls back
to `"project"`.

`src/components/BridgeSidebar.test.tsx` / `.interaction.test.tsx` — right-edge
classes, no `pl-24` on the rail, no Work/Code, no Needs-you, Repositories
label, both plain and project chats listed, Automations fires
`onOpenAutomations`.

`src/components/AppTitleBar.test.tsx` — flush has `pl-24` and does not have
the trailing window-control inset.

`src/navigationHistory.test.ts` — `automations` is a distinct view.

`src/workWiring.test.ts` — rail is not handed Work-board props; Work view
still exists in `App.tsx`.

`src/App.test.tsx` — `App.tsx` contains neither `NewChatDialog` nor
`RightRailPreview`.

## Unchanged (every phase)

- Hidden session kinds, `GROUP_ROW_CAP = 12`, search bypass, git badge,
  `chatListTime`, collapse/resize/drawer.
- Semantic tokens; `src/designSystem.test.ts` stays green.
- No new `.css` file.

## Manual Tests

Vite `?preview=right-rail` remains for visual comparison. Real app:

1. Rail on the right; traffic lights on the canvas; resize from the left edge.
2. New Chat lands in the last repo, no dialog.
3. Automations is Automations, with Browse catalog as the only way to Agents.
4. Composer strip: repo, selectable local branch, On branch / Isolated
   worktree, This Mac. Dirty and active workspaces refuse checkout.
5. Fullscreen: flush rectangle.
