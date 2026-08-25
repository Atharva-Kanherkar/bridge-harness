# Plan: right rail, repo-only chats, Cursor-style new chat

Preview shipped 2026-08-25; detailed plan locked the same day. Review on
2026-08-25 locked the remaining §10 questions to the draft defaults (see
that section) and implementation started. This document is the source of
truth. Do not retarget `main`.

Preview URL (Vite already serving):

```
http://127.0.0.1:1420/?preview=right-rail
```

The shipping app at `/` is unchanged. The preview is a self-contained React
page gated by `?preview=right-rail` in `main.tsx` — real Tailwind tokens,
lucide icons, and window chrome, not a canvas mock.

---

## 1. Goal and non-goals

**Goal.** After visual sign-off:

- the sidebar moves to the **right** edge;
- fullscreen renders a **flush rectangle** — no inset rounded card, no gutter;
- **Work / Code is gone**; one list, **Repositories**, every chat under its repo;
- the rail's **Automations** row opens **Automations only** — never the
  Agents / Plugins / Skills catalog;
- **New Chat** skips the "Where should it run?" dialog and lands in the
  current / last-used repo, with a composer context strip: repository, branch,
  worktree, and agent host (**This Mac / Cloud / SSH**).

**Non-goals.**

- No protocol/backend change. Cloud and SSH hosts are drafted UI; only local
  execution works. A later protocol plan wires them.
- No PR against `main` until a human asks.
- No revival of `SHOW_CURSOR_SIDEBAR_MOCK` / `CursorSidebarMock.tsx`.
- The Work board (`WorkView`) is not deleted — it just loses its rail row
  (reversible, §10 Q4).
- Creating or tracking remote branches from the composer. The branch chip lists
  existing local refs and switches only while the workspace is clean, inactive,
  and not using an isolated worktree.

---

## 2. Branching and PR rules (non-negotiable)

Same integration branch as the rail restyle:

```
main
  └── (do not PR here)
feat/cursor-sidebar-dev            ← review base. No PR to main.
  ├── feat/right-rail-p0-plan       ← this document + the look-at preview
  ├── feat/right-rail-p1-contract
  ├── feat/right-rail-p2-shell
  ├── feat/right-rail-p3-list
  ├── feat/right-rail-p4-automations
  ├── feat/right-rail-p5-composer
  └── feat/right-rail-p6-delete-preview
```

1. Every `gh pr create` uses `--base feat/cursor-sidebar-dev`. Never `main`.
2. Branch each phase from the **current tip** of `feat/cursor-sidebar-dev`
   (pull first). No stacking on unmerged siblings.
3. After a phase PR opens, merge it into the integration branch so the next
   phase can start; the PR stays as the review record.
4. Conventional Commits; no issue/PR numbers in subjects. PR bodies may say
   `Part of #<epic>`.
5. `bun run build` plus the phase's named Vitest files must be green before
   the PR opens. Full `bun run test` (including cargo) is the Phase 6 gate.
6. No force-push of `feat/cursor-sidebar-dev`; no amending published commits.

---

## 3. Why the current UI does the wrong thing

| What you see | Where it comes from |
|---|---|
| Rail on the left | `App.tsx` renders `BridgeSidebar` before the canvas; the aside is `fixed inset-y-0 left-0 … border-r`, drawer slides in with `-translate-x-full`. |
| Work / Code switch | `ScopeSwitch` in `BridgeSidebar.tsx`; `ChatScope` + `CHAT_SCOPE_KEY` (`bridge.sidebar.scope`) in `sidebarChats.ts`. |
| Chats vs Repositories | The section label flips on scope (`scope === "code" ? "Repositories" : "Chats"`); Work scope groups by date. |
| Automations opens Agents | The row calls `onOpenMarketplace` → `setView("marketplace")` → `MarketplaceScreen`, whose state starts `useState<Resource>("agents")`. Automations is the fourth tab. |
| New Chat asks "Where should it run?" | `onOpenNewChat={() => setModal("chat")}` → `NewChatDialog` (project radio + worktree checkbox) → `startChat(choice)`. |
| No host picker | Nothing in the composer names a host. `ComposerPill` has no context strip. |
| Rounded canvas in fullscreen | The 16px radius is the **native macOS window** (`--window-radius` exists for nested chrome like `rounded-window-control`, not the canvas). What reads as an inset card in fullscreen must be verified live in Phase 2 — see §6. |

---

## 4. Locked decisions

The preview implements these. Any row can be overridden by your preview
feedback (§10); until then, phase PRs follow this table.

| # | Decision | Detail |
|---|---|---|
| 1 | **Rail on the right.** | Canvas first in the flex row, aside last. `border-l` not `border-r`. Resize handle on the rail's **left** edge. Mobile drawer slides in from the right. Panel icon `PanelRight`. |
| 2 | **Traffic lights stay top-left, on the canvas.** | Windowed: the canvas's flush `AppTitleBar` takes the `pl-24` traffic-light inset. The rail chrome strip is panel + history chevrons only, no inset. |
| 3 | **Windowed rounding stays.** | The window is the native 16px rounded rectangle. Nothing new to build; nothing to break. |
| 4 | **Fullscreen is a flush rectangle.** | Canvas + rail fill the screen edge-to-edge. No nested radius, no gutter. Verified in `bun run tauri dev`, not just Chrome (§6). |
| 5 | **Work / Code deleted.** | `ScopeSwitch` and the whole `ChatScope` persistence path go away. One list for every visible top-level chat. |
| 6 | **One section: Repositories.** | Default `groupBy` becomes `"project"`. No-workspace chats sit under the existing `NO_PROJECT_GROUP_KEY` bucket. `SidebarFilterMenu` keeps date/agent/status grouping as opt-ins (`allowProjectGrouping` is always true). |
| 7 | **Needs-you count leaves the rail.** | The Work/Code scope switch and `workNeedsYouCount` badge are gone. `WorkView` is reachable from a **Work board** action row now that those two "Work" meanings no longer collide. |
| 8 | **Automations opens Automations.** | New `AppView` `"automations"` renders `AutomationsPanel` alone. The rail row never touches `setView("marketplace")`. The four-tab `MarketplaceScreen` survives as a view; its interim entry is a quiet "Browse catalog" link on the Automations screen header (pending Q5). |
| 9 | **New Chat skips the dialog.** | Resolve a workspace (§5), create the session there, focus the composer. `NewChatDialog.tsx` is deleted in Phase 5 — nothing else imports it. |
| 10 | **Composer context strip.** | New `ComposerContextStrip` above the input: repository (menu), branch (local-ref menu, pre-first-turn only), worktree (toggle, pre-first-turn only), host (menu). Icons: `FolderGit2`, `GitBranch`, `GitFork`, `Laptop` / `Cloud` / `Terminal`. |
| 11 | **Host is honest UI.** | `This Mac` selected and functional. Cloud and SSH render as **disabled** menu rows with hint copy ("Not wired up yet") — visible in the menu but not selectable, unlike the preview where they toggle for look-and-feel. No persistence while only one host works. |
| 12 | **Mission Control + account settings.** | Mission Control replaces Customize; Projects and Memory follow it. The footer is a local-user row whose gear opens Settings. |
| 13 | **Preview stays URL-gated until Phase 6 deletes it.** | `App.tsx` never imports it; `App.test.tsx` guards that. |

---

## 5. New Chat, exactly

Today's paths in `App.tsx`:

- rail New Chat → `setModal("chat")` → `NewChatDialog` → `startChat(choice)` →
  `createChat` (no project) or `createWorkspaceSession(workspaceId, worktree)`;
- Welcome composer → `startChatOrShortcut` → `openNewChat` (direct chat).

Target (`startChatInCurrentRepo`, Phase 5):

1. Resolve the workspace, first match wins:
   1. the active session's `workspaceId`;
   2. `localStorage["bridge.chat.lastWorkspaceId"]`, validated against
      `state.workspaces` (stale ids fall through);
   3. the only workspace, when exactly one exists;
   4. none → fall back to today's `openNewChat()` direct chat.
2. Repo path: `bridgeApi.createWorkspaceSession(workspaceId, false)` —
   worktree **off** by default — then `openSession(created.id)`.
3. Write `bridge.chat.lastWorkspaceId` every time a workspace session is
   opened or created, so "the repo you were just in" is always current.
4. The composer strip shows that repo, its `Workspace.branch`, the worktree
   chip, and the host chip. The worktree chip can flip **only before the first
   turn** and only when `workspace.projectId` is set (same `canWorktree` rule
   the dialog had). After the first turn it is a static label.
5. Repo chip menu: active only before the first turn. Choosing a different
   repo retargets by creating a fresh session in the chosen repo and dropping
   the empty one (an empty session holds nothing worth migrating).

`$harness` shortcuts and the Welcome hero keep their existing direct-chat
behavior — the strip appears there too once a repo is resolvable, but the
Welcome flow itself is not this plan's target.

---

## 6. Shell geometry and the fullscreen question

Windowed:

```
┌──────────── native window, radius 16 ────────────────────┐
│ ● ● ●  canvas (flush title bar, pl-24)       │  rail     │
│        …content…                             │  actions  │
│        composer + context strip              │  repos    │
└──────────────────────────────────────────────┴───────────┘
```

Fullscreen:

```
┌──────────── flush rectangle, radius 0 ───────────────────┐
│ title bar spans full width                                │
│ canvas (full remaining width)                │  rail      │
└──────────────────────────────────────────────┴───────────┘
```

The radius is native, so macOS already squares the window in true fullscreen.
What the screenshot showed as an inset rounded card must be identified live in
Phase 2 under `bun run tauri dev` (the internal `fullscreen` state toggles at
`App.tsx` around the Escape/shortcut handler, separate from native
fullscreen). Candidate sources, in order of suspicion:

1. a gutter left by the `u-vibrancy-sidebar` wash meeting the opaque
   `u-vibrancy-canvas` at the seam;
2. `corner-shape: squircle` chrome (`rounded-window` / `rounded-window-control`)
   drawing a nested arc where the canvas meets the window edge;
3. the internal-fullscreen layout (`flex-col` + title bar) leaving padding
   around the canvas row.

Acceptance for Phase 2: in both internal and native fullscreen, canvas + rail
form an edge-to-edge rectangle — no radius, no gutter, no darker frame.

---

## 7. Phases

Every phase: PR against `feat/cursor-sidebar-dev`; `bun run build` green; the
named test files green.

### Phase 0 — plan + preview (this PR)

Files: this document; `src/previews/RightRailPreview.tsx` (+ its test);
`src/main.tsx` query gate; `src/App.test.tsx` guard extended with
`RightRailPreview`.

Verify: `bunx vitest run src/previews/RightRailPreview.test.tsx src/App.test.tsx`.

### Phase 1 — test contract (after your preview pass)

Write `testing/feat-right-rail-composer.md`, folding in your answers to §10.
It **supersedes** these rules in earlier contracts, by name:

- `testing/feat-real-cursor-rail.md`: rail side, `pl-24` on the rail strip,
  scope switch and Needs-you placement, "Chats" label, Automations →
  Marketplace mapping, New Chat → dialog mapping.
- `testing/feat-sidebar-redesign.md` / `feat-projects-screen-compact-header.md`:
  the scope-persistence and Needs-you-row data rules.

It **keeps**: hidden session kinds, filter persistence (`CHAT_VIEW_KEY`),
`GROUP_ROW_CAP = 12` with search bypassing the cap, git badge from
`Workspace.branch`, `chatListTime`, collapse/resize/drawer/drag behavior.

### Phase 2 — rail to the right, fullscreen flush (`feat/right-rail-p2-shell`)

`BridgeSidebar.tsx`:

- aside: `left-0` → `right-0`; drawer `-translate-x-full` → `translate-x-full`;
  `border-r border-sidebar-border` → `border-l border-sidebar-border`.
- chrome strip: drop `pl-24` (the strip no longer hosts the traffic-light
  corner). Panel button keeps the left slot, chevrons keep `ml-auto` —
  matching the preview.
- resize handle: `right-0` → `left-0`, `after:right-0` → `after:left-0`, and
  the drag math inverts — a right-anchored rail grows as the pointer moves
  left: `startWidth - (moveEvent.clientX - startX)` replacing
  `startWidth + moveEvent.clientX - startX`.

`WindowNavButtons.tsx`: `PanelLeft` → `PanelRight` (same 15px / 1.5 stroke).

`AppTitleBar.tsx`: flush variant takes the traffic-light inset (`pl-2` →
`pl-24`); the mobile nav button icon flips to `PanelRight`.

`App.tsx`: windowed row renders canvas first, `{!fullscreen && sidebar}`
after; internal-fullscreen row renders content first, `{fullscreen && sidebar}`
last. Then the §6 live audit and whatever one-line fix it names.

Tests: update `BridgeSidebar.test.tsx` ("puts panel beside the traffic
lights…" and "…does not inset for traffic lights" become right-rail
assertions), `AppTitleBar.test.tsx` ("leaves the traffic lights their corner"
moves to the flush variant), `WindowNavButtons.test.tsx` icon/labels.
`SessionToolbar.test.tsx` should stay green untouched.

Verify:

```bash
bunx vitest run src/components/BridgeSidebar.test.tsx src/components/BridgeSidebar.interaction.test.tsx src/components/AppTitleBar.test.tsx src/components/WindowNavButtons.test.tsx
bun run tauri dev   # fullscreen flush check, drawer from the right, resize drag
```

### Phase 3 — one Repositories list (`feat/right-rail-p3-list`)

`sidebarChats.ts`:

- delete `ChatScope`, `CHAT_SCOPE_KEY`, `readChatScope`, `writeChatScope`,
  `chatScope`, `inScope`;
- `DEFAULT_CHAT_VIEW.groupBy` → `"project"`;
- `readChatView` keeps honoring a persisted choice (someone who picked
  `Group by → Date` keeps it); the old Work-scope date-correction rule goes
  away with the scope itself.

`BridgeSidebar.tsx`: delete `ScopeSwitch`, the `scope` state and its
follow-the-opened-chat effect, and the Needs-you block; the section label is
always "Repositories"; `allowProjectGrouping` is always `true`; empty-state
copy becomes one sentence.

`App.tsx`: drop `workBoardActive`, `workNeedsYouCount`, `onOpenWorkBoard`
from the rail's props. `view === "work"` stays routable.

Tests: `sidebarChats.test.ts` loses the scope cases and gains a
default-grouping case; `BridgeSidebar.test.tsx` / `.interaction.test.tsx`
drop scope/Needs-you expectations.

Verify:

```bash
bunx vitest run src/components/sidebarChats.test.ts src/components/BridgeSidebar.test.tsx src/components/BridgeSidebar.interaction.test.tsx src/components/SidebarFilterMenu.test.tsx
```

### Phase 4 — Automations view (`feat/right-rail-p4-automations`)

- `navigationHistory.ts`: `AppView` gains `"automations"`.
- `App.tsx`: lazy-import `AutomationsPanel` (same pattern as
  `MarketplaceScreen`); render it for `view === "automations"`; `chromeTitle`
  gains "Automations"; rail props rename `onOpenMarketplace` →
  `onOpenAutomations`, `marketplaceActive` → `automationsActive`.
- Automations screen header gains the quiet "Browse catalog" text link →
  `setView("marketplace")` (interim entry for Agents / Plugins / Skills,
  pending Q5).
- `MarketplaceScreen` itself is untouched.

Tests: `BridgeSidebar.interaction.test.tsx` asserts the row fires
`onOpenAutomations`; a small `navigationHistory` case covers the new view in
back/forward.

Verify:

```bash
bunx vitest run src/components/BridgeSidebar.interaction.test.tsx src/navigationHistory.test.ts
# manual: click Automations → h1 is "Automations"; no segmented Agents/Plugins/Skills/Automations control
```

### Phase 5 — New Chat + composer strip (`feat/right-rail-p5-composer`)

- `App.tsx`: `startChatInCurrentRepo()` per §5; `bridge.chat.lastWorkspaceId`
  written on open/create of any workspace session; rail's `onOpenNewChat`
  points at it; the `modal === "chat"` branch, `startChat`, and
  `NewChatChoice` plumbing are removed.
- Delete `NewChatDialog.tsx` and `NewChatDialog.test.tsx`.
- New `src/components/ComposerContextStrip.tsx` (+ test): the four chips per
  locked decisions 10–11, rendered above `ComposerPill` in the session dock
  and on the Welcome hero when a repo resolves. Menus via the existing
  `menu-panel` primitives; chips are `h-7` ghost buttons like the rail's
  action rows.

Tests: `ComposerContextStrip.test.tsx` (chip rendering, host menu disabled
rows, worktree lock after first turn); App-level wiring assertions.

Verify:

```bash
bunx vitest run src/components/ComposerContextStrip.test.tsx src/App.test.tsx
rg -n "NewChatDialog|NewChatChoice" src/ && echo FAIL || echo clean
# manual: rail New Chat → lands in last repo, no dialog; worktree chip flips before first send only
```

### Phase 6 — delete the preview (`feat/right-rail-p6-delete-preview`)

- Delete `src/previews/` and the `main.tsx` query gate.
- Keep the `App.test.tsx` guard line (it pins `App.tsx`, which never imported
  the preview).

Verify:

```bash
rg -n "preview=right-rail|RightRailPreview" src/   # only the App.test.tsx guard may remain
bun run build && bun run test
```

---

## 8. Risks and watch-outs

- **Concentric-corner chrome.** `UsageWidget` uses `rounded-window-control`
  and the title bar's `pr-[var(--window-control-inset)]` so its top-right arc
  is concentric with the window. With the rail on the right, the title bar's
  right edge is an interior seam, not the window corner — the window's real
  top-right corner belongs to the rail. Phase 2 must eyeball this in Tauri;
  the likely fix is dropping the inset on the title bar and letting the rail
  strip own it.
- **Superseded contracts.** `feat-sidebar-redesign.md` and
  `feat-projects-screen-compact-header.md` lock scope persistence and the
  Needs-you row. Phase 1 must supersede those rules *by name* or later agents
  will "fix" the rail back.
- **`workWiring.test.ts`** bans `openSession`/`setSelectedSessionId` patterns
  in certain files — check before wiring `startChatInCurrentRepo`.
- **Vitest picks up `.claude/worktrees/**` copies** in bare `vitest run` output
  noise; run the named files per phase and trust the Phase 6 full run.
- **`data-tauri-drag-region` is inert in Chrome.** Window-drag checks happen
  in `bun run tauri dev`, not in the Vite browser.
- **Design-system guards** (`designSystem.test.ts`): no palette classes, no
  white/black alpha, no blur on resting surfaces — the new strip and menus
  must use tokens (`bg-card`, `border-border`, `u-glass-popover`).

---

## 9. Acceptance checklist (end state)

- [ ] Rail on the right in windowed, internal-fullscreen, and drawer modes;
      resize drag works with the handle on the rail's left edge.
- [ ] Traffic lights sit over the canvas title bar; nothing overlaps them.
- [ ] Fullscreen: edge-to-edge rectangle, no rounded inset, no gutter.
- [ ] No `ScopeSwitch`, no `ChatScope`, no `bridge.sidebar.scope` reads.
- [ ] One "Repositories" section; default grouping is by project;
      `GROUP_ROW_CAP = 12` and search-bypass intact.
- [ ] Rail Automations row → Automations screen; the Agents-first
      `MarketplaceScreen` is only reachable from the interim catalog link.
- [ ] Rail New Chat → session in current/last repo; `NewChatDialog` deleted.
- [ ] Composer strip: repo menu, branch display, worktree toggle
      (pre-first-turn), host menu with disabled Cloud/SSH rows.
- [ ] `src/previews/` deleted; `bun run build` and full `bun run test` green.

---

## 10. Open questions — locked 2026-08-25

Building started without a second preview pass, so these stay the draft
defaults. Reverse any row in a later message and the contract follows.

1. **Host chips:** one menu. Three always-visible icons would crowd the strip
   next to repo / branch / worktree.
2. **Worktree:** toggle chip, not a dropdown.
3. **"Run in Cloud" shortcut:** dropped. Cloud is a disabled menu row; a
   shortcut that cannot run would be a lie.
4. **Needs you:** stays off the rail. `WorkView` remains routable.
5. **Agents / Plugins / Skills:** quiet "Browse catalog" link on the
   Automations screen header. No extra rail row.
6. **Fullscreen:** flush to the screen edge. No nested 16px card.

### Review findings folded in

- `NewChatDialog` is only mounted from `App.tsx` (Projects uses
  `OrchestratorCreateDialog`). Deleting it in Phase 5 is safe.
- `workWiring.test.ts` currently requires the rail to receive
  `workBoardActive` / `workNeedsYouCount` / `onOpenWorkBoard`. That file
  must be updated in Phase 3, not left to fail.
- Windowed title bar is now the traffic-light edge (`flush` gets `pl-24`).
  Fullscreen title bar still spans the whole window, so it keeps
  `pr-[var(--window-control-inset)]`; the flush (windowed) variant drops
  that inset because its right edge is the rail seam, not the window corner.
- Default `groupBy: "project"` makes day-header tests need an explicit
  persisted `groupBy: "date"` rather than relying on the default.
