# Plan: turn the Cursor-style sidebar mock into the real, working rail

Reviewed and locked 2026-08-25. This is the document every later phase PR
follows. Do not retarget `main`. Do not reopen the design decisions in §5
unless a later PR's description explicitly supersedes one.

---

## 0. Review of the draft plan (what changed)

The first draft was executable but had holes that would make a later agent
ship the wrong thing. This revision is the source of truth.

| Finding | Why it matters | Resolution |
|---|---|---|
| No branching model | User asked that **no PR target `main`**. The draft only said "don't push unless asked." | Long-lived integration branch `feat/cursor-sidebar-dev`. Every phase PR uses `--base feat/cursor-sidebar-dev`. |
| Phase 1 "wait for sign-off" vs "start building" | Blocking on unanswered questions would stall the work. | Decisions in §5 are **locked** to the draft's recommended defaults, with two corrections below. |
| Phase 0 mixed chrome + mock-on | Committing `SHOW_CURSOR_SIDEBAR_MOCK = true` hides `BridgeSidebar` in windowed mode, so Phases 3–5 cannot be verified. | Integration branch keeps the mock **file** as a visual reference. The app ships the real rail. Flag stays off; Phase 6 deletes the file. |
| Staged chrome already deletes the mock from `App.tsx` | That is Phase 6 work hiding inside Phase 2. | Phase 2 may stop *rendering* the mock (so the real rail is what you see). It must **not** delete `CursorSidebarMock.tsx` or its design-system allowlist entry. Phase 6 does that. |
| `relativeTime` already exists in two modules | Adding a third copy in `utils.ts` is the design bug the draft warned about. | New helper lives in `sidebarChats.ts` as `chatListTime`, compact (`2h`, `4m`, `1d`) to match the mock. Do not import `workDashboard.relativeTime` (`2h ago`) or `workFacts.relativeTime`. |
| `GROUP_ROW_CAP` of 4 vs 12 | The mock caps at 4 because it only has a handful of fake chats. Real history is dozens of same-named orchestrators. Existing contract `testing/feat-sidebar-redesign.md` already locked 12. | Keep **12**. Search still bypasses the cap. |
| Default grouping | The mock always shows "Repositories". Real Work-scope chats have no project, so a default of `groupBy: "project"` would render one "No project" bucket. Existing `DEFAULT_CHAT_VIEW.groupBy` is `"date"`. | Do **not** change the default. Style project groups when the filter says so. Code-scope users pick `Group by → Project`. |
| Customize → Settings duplicates the footer Settings row | Draft mapped Customize to Settings, then kept Settings in the footer. | Action row **Customize** → `onOpenSettings`. Phase 5 drops the footer Settings row (Customize already reaches it). Footer keeps **Projects** and **Memory**. **Automations** already reaches Marketplace, so Phase 5 drops the footer Marketplace row too. |
| Scope switch and Needs-you have no mock counterpart | They are the only way to split Work/Code and to open the Work board. Existing contract `testing/feat-projects-screen-compact-header.md` requires both. | Both survive. Placement locked in §5. |
| `Workspace.branch` is optional | Git badge cannot be "session is on a branch" — Session has no branch field. | Git badge when the chat's `workspaceId` resolves to a workspace whose `branch` is a non-empty string. |
| Phase 0 screenshots as a hard gate | No screenshot fixture dir is in repo convention, and the model wizard blocks Vite. | Optional. Visual compare is the Phase 7 manual QA, not a merge blocker. |
| Existing contracts | `testing/feat-sidebar-redesign.md` and `testing/feat-projects-screen-compact-header.md` already lock data behavior. This work restyles; it must not silently rewrite those rules. | Phase 1 contract **supersedes visual** rules only. Data rules (scope, hidden session kinds, filter persistence, cap 12, Needs-you) stay. |

---

## 1. Goal and non-goals

**Goal.** One sidebar, `src/components/BridgeSidebar.tsx`, that:

- looks like `src/components/CursorSidebarMock.tsx`: chrome strip (panel +
  back/forward), action rows, repo-styled groups, compact chat rows, dense footer;
- is fully real: live sessions, live workspaces, working navigation history,
  filter, collapse/resize/mobile drawer, window dragging;
- ends with `CursorSidebarMock.tsx` deleted and no `SHOW_CURSOR_SIDEBAR_MOCK`
  anywhere in `src/`.

**Non-goals.** No backend/protocol changes. No cloud-sync feature. No theme
work beyond existing tokens. No PR against `main` until a human asks.

---

## 2. Branching and PR rules (non-negotiable)

```
main
  └── (do not PR here)
feat/cursor-sidebar-dev          ← integration / review base. No PR to main.
  ├── feat/cursor-sidebar-p0-plan
  ├── feat/cursor-sidebar-p1-contract
  ├── feat/cursor-sidebar-p2-chrome
  ├── feat/cursor-sidebar-p3-actions
  ├── feat/cursor-sidebar-p4-list
  ├── feat/cursor-sidebar-p5-footer
  └── feat/cursor-sidebar-p6-delete-mock
```

1. **Base branch for every `gh pr create` is `feat/cursor-sidebar-dev`.** Never
   `--base main`. Never `--base fix/cursor-level-dark-ground`.
2. Create each phase branch from the **current tip** of `feat/cursor-sidebar-dev`
   (pull first). Do not stack PRs on top of an unmerged sibling.
3. After a phase PR is opened, merge it into `feat/cursor-sidebar-dev`
   (`gh pr merge --merge` if checks allow; otherwise merge locally and push the
   integration branch) so the next phase can start. The open PR is the review
   record.
4. Commit messages are Conventional Commits. Do not cite issue or PR numbers
   in the subject. The PR body may say `Part of #<epic>`.
5. `bun run build` and the frontend tests named in that phase's Verify block
   must be green before the PR is opened. Full `bun run test` (including cargo)
   is required at Phase 7, not at every phase.
6. Do not force-push `feat/cursor-sidebar-dev`. Do not amend published phase
   commits.

Integration branch starting point: tip of `fix/cursor-level-dark-ground` at the
moment this work began (wallpaper tint + near-black ground already on that
line, plus `origin/main` merged in). That is deliberate — the rail is being
restyled in the visual environment it will ship in.

---

## 3. Ground rules (repo conventions)

- **Tailwind CSS v4 only.** Utilities in JSX; tokens over hardcoded values; no
  new `.css` files; no inline `style` for anything a utility can express.
- Tests are colocated `*.test.ts(x)` under Vitest. Component tests start with
  `// @vitest-environment jsdom` and mount via `react-dom/client` + `act`.
- Ignore Vitest hits under `.claude/worktrees/**` and `.worktrees/**`.
- Sizeable feature ⇒ `testing/*.md` contract first (Phase 1).
- Vite: `bun run dev` → `http://127.0.0.1:1420/`. Kill whatever owns 1420 if
  it is already taken.
- The **Set up Bridge models** wizard blocks a fresh load; click "Use
  recommended defaults".
- Desktop: `bun run tauri dev`. One owner per data dir — kill stray `bridged`
  / `bridge-deck` if the daemon complains.
- `data-tauri-drag-region` only works in the Tauri app, never in Chrome.

---

## 4. Mock → real mapping

| # | Mock element | Real counterpart | Phase |
|---|---|---|---|
| 1 | Chrome strip: panel, back, forward (dead) | `WindowNavButtons` spread: panel after traffic lights (`pl-24`), chevrons `ml-auto`. Wired to `navigationHistory` in `App.tsx`. Fullscreen moves them to `AppTitleBar`. | 2 |
| 2 | `New Chat` action row | `onOpenNewChat` → NewChatDialog. Mock's `ActionRow` look (h-7, ghost, 13px), not the current filled primary button. | 3 |
| 3 | `Search` action row | Toggles existing `searchOpen` + `query` + `filterChats`. Moves off the Chats header. | 3 |
| 4 | `Automations` action row | `onOpenMarketplace` (MarketplaceScreen hosts `AutomationsPanel`). | 3 |
| 5 | `Customize` action row | `onOpenSettings`. | 3 |
| 6 | `Repositories` header + filter + new-folder | Section label (Work: `Chats`; Code: `Repositories`) + `SidebarFilterMenu` + new-folder → `onOpenProjects`. | 4 |
| 7 | Repo groups, collapsible | Existing `groupChats` + `foldedGroups`. Folder icon per project group; `Home` icon for the no-project group. | 4 |
| 8 | Title, git icon, cloud icon, time | Title = `chatName`; keep `StatusDot`; `chatListTime(chatTimestamp(chat), now)`; git icon iff workspace.branch is non-empty; **no cloud icon**. | 4 |
| 9 | `More` after 4 rows | Keep `GROUP_ROW_CAP = 12` and `shownInFull`. Copy stays `Show N more`. Search bypasses the cap. | 4 |
| 10 | Avatar + hardcoded name + gear | No account object. Footer = Projects + Memory only after Phase 5. No invented name, no conic-gradient avatar. | 5 |
| — | Scope switch (absent from mock) | Survives. Below action rows, above Needs-you. Collapsed: icon-only tabs as today. | 3 |
| — | Needs-you (absent from mock) | Survives. Work scope only. Count badge hidden at zero. `aria-current` while the Work board is open. | 3 |
| — | Collapse / resize / drawer / drag | Already in `BridgeSidebar`. Must keep working every phase. | all |

---

## 5. Locked decisions

These are closed. A later PR may not silently pick the other option.

1. **Customize → Settings** (`onOpenSettings`), not Projects.
2. **Cloud icon dropped.** No real sync signal.
3. **Git badge** only when `workspaces.find(w => w.id === chat.workspaceId)?.branch` is a non-empty string.
4. **`GROUP_ROW_CAP` stays 12.**
5. **Default `groupBy` stays `"date"`.** Do not force project grouping.
6. **Scope switch** sits below the action rows and above Needs-you.
7. **Needs-you** stays, Work scope only, between the scope switch and the list.
8. **Footer after Phase 5:** Projects, Memory. Automations and Customize cover Marketplace and Settings. Collapsed rail: the same two as icon-only, plus the action icons from Phase 3.
9. **No fake account row.**
10. **`SHOW_CURSOR_SIDEBAR_MOCK` is not turned back on.** The mock file remains until Phase 6 for side-by-side reading, not for rendering.
11. **`chatListTime`** lives in `sidebarChats.ts`, compact (`just now` / `Nm` / `Nh` / `Nd`). Unit-tested there.
12. **Type scale** stays inside the existing rail rules: 13px rows, 11px section labels, no `text-[8px]`/`text-[9px]`/`text-[9.5px]`. The mock's 13px / 12px / 11px is compatible.

---

## 6. Current code facts (do not rediscover)

- `src/components/sidebarChats.ts` already owns `filterChats`, `groupChats`
  (including `groupBy: "project"`), `GROUP_ROW_CAP`, status buckets, persisted
  `ChatView` / `ChatScope`.
- `BridgeSidebar` already has collapse, resize, mobile drawer, Needs-you,
  scope switch, filter menu, fold, cap, footer routes.
- Window chrome (`WindowNavButtons.tsx`, `navigationHistory.ts`, sidebar
  `pl-24` strip, fullscreen `AppTitleBar` leading/trailing) is written in the
  working tree and lands in **Phase 2**.
- `CursorSidebarMock.tsx` is still tracked. `SHOW_CURSOR_SIDEBAR_MOCK` is
  `false` on the integration-branch parent. The mock is a look-at file, not
  the running UI.
- `Workspace.branch` is optional on the generated protocol type.

---

## Phase 0 — Integration branch + reviewed plan

**Goal:** a named integration branch that later PRs target, and this document
on it.

1. Create `feat/cursor-sidebar-dev` from the then-current
   `fix/cursor-level-dark-ground` tip. Push it. Do **not** open a PR to `main`.
2. Branch `feat/cursor-sidebar-p0-plan`. Add this file. Open a PR against
   `feat/cursor-sidebar-dev`.
3. Create the GitHub epic (checklist of phases) so later PRs can say
   `Part of #<n>`.

**Verify:**

```bash
git rev-parse --abbrev-ref HEAD   # a p0 branch, not main
gh pr view --json baseRefName -q .baseRefName   # feat/cursor-sidebar-dev
```

**Review gate:** the PR exists, base is the integration branch, this document
is the whole of the diff.

## Phase 1 — Lock the test contract

**Goal:** `testing/feat-real-cursor-rail.md` states every behavior in §4–§5 as
a testable assertion, including data source and the test file that will cover
it.

Must include:

- Chrome strip layout and fullscreen move.
- Action rows and their handlers.
- Scope switch and Needs-you survival.
- Chat row anatomy (dot, title, git, time, no cloud, no harness/model in
  visible text — hover `title` still carries harness/model per the older
  contract).
- Cap 12, search bypass, fold.
- Footer after Phase 5.
- Mock deletion checks for Phase 6.
- Explicit "unchanged" list pointing at `testing/feat-sidebar-redesign.md` and
  `testing/feat-projects-screen-compact-header.md` for data rules.

**Verify:** file exists; every §4 row has a corresponding test bullet.

**Review gate:** contract PR merged into `feat/cursor-sidebar-dev`.

## Phase 2 — Chrome strip

**Goal:** `[lights] [panel] …… [←][→]` on the real rail.

1. `WindowNavButtons` (`spread` puts panel left, chevrons `ml-auto`).
2. Expanded rail: `h-11` strip, `pl-24`, `data-tauri-drag-region="deep"`.
3. Collapsed rail: panel-only, centered, no `pl-24`.
4. `showWindowNav={!fullscreen}`. Fullscreen: `AppTitleBar` gets
   `leading={WindowPanelButton}` and `trailingNav={WindowHistoryChevrons}`.
5. `navigationHistory.recordPlace` in `App.tsx`; chevrons disable at edges.
6. Session toolbar does not also `pl-24` in fullscreen (the title bar owns
   that inset).
7. Keep `CursorSidebarMock.tsx` and its design-system allowlist. Stop
   *rendering* it from `App.tsx` if that is not already the case.

**Verify:**

```bash
bunx vitest run src/components/WindowNavButtons.test.tsx \
  src/components/BridgeSidebar.test.tsx src/components/AppTitleBar.test.tsx \
  src/navigationHistory.test.ts src/App.test.tsx src/designSystem.test.ts
bunx tsc -b --pretty false
```

**Review gate:** tests named above green; collapsed rail still 68px and does
not crowd the traffic lights.

## Phase 3 — Action rows

**Goal:** the block under the chrome strip is the mock's action list, with
real handlers.

1. Local `ActionRow` (h-7, 13px, icon-left, ghost hover).
2. Rows in order: New Chat, Search, Automations, Customize.
3. Scope switch under the rows. Needs-you under the switch (Work only).
4. Search control moves off the Chats header; the header keeps the filter
   menu.
5. Collapsed: four icon-only 9×9 buttons with `title`/`aria-label`, then
   icon-only scope, then Needs-you icon.
6. Tests: handlers fire; Needs-you still reachable; collapsed icon-only;
   existing interaction tests still pass.

**Verify:**

```bash
bunx vitest run src/components/BridgeSidebar.test.tsx \
  src/components/BridgeSidebar.interaction.test.tsx
bunx tsc -b --pretty false
```

## Phase 4 — List body

**Goal:** compact, indent-grouped chat rows from live data.

1. Section label: `Chats` in Work, `Repositories` in Code. Filter menu +
   new-folder (`onOpenProjects`) on the right.
2. Group headers: folder icon (Home for no-project), fold on click.
3. Rows 26px, `pl-7` when grouped, status dot, truncated title, git badge
   per §5.3, `chatListTime` right-aligned. No cloud. No model on the row
   (keep it in `title=`).
4. Cap + `Show N more`. Search expands groups and bypasses the cap.
5. `chatListTime` tests in `sidebarChats.test.ts`. Row assertions in
   `BridgeSidebar.test.tsx`.

**Verify:**

```bash
bunx vitest run src/components/sidebarChats.test.ts \
  src/components/BridgeSidebar.test.tsx \
  src/components/BridgeSidebar.interaction.test.tsx
bunx tsc -b --pretty false
```

## Phase 5 — Footer and density

**Goal:** mock density, real routes, no fake person.

1. Footer: Projects, Memory only.
2. Sweep spacing/typography against the mock (row heights, 13px labels,
   `tracking-[-0.008em]`, muted icon weight 1.5).
3. Collapse, resize, drawer, drag regions still work; interaction tests
   updated for the shorter footer.

**Verify:**

```bash
bunx vitest run src/components/BridgeSidebar.test.tsx \
  src/components/BridgeSidebar.interaction.test.tsx src/designSystem.test.ts
bunx tsc -b --pretty false
```

## Phase 6 — Delete the mock

**Goal:** one sidebar in the tree.

1. Delete `src/components/CursorSidebarMock.tsx`.
2. Remove it from `COLOR_LITERAL_ALLOWLIST`.
3. `App.test.tsx` absence guard: source contains neither
   `SHOW_CURSOR_SIDEBAR_MOCK` nor `CursorSidebarMock`.

**Verify:**

```bash
rg -n "CursorSidebarMock|SHOW_CURSOR_SIDEBAR_MOCK" src/ && echo FAIL || echo clean
bunx vitest run src/App.test.tsx src/designSystem.test.ts src/components/BridgeSidebar.test.tsx
bunx tsc -b --pretty false
```

## Phase 7 — Full verification (no PR unless asked)

Runs on `feat/cursor-sidebar-dev` after Phase 6 lands. Does not open a PR
to `main`.

1. `bun run build`
2. `bun run test`
3. Manual QA in Tauri: drag, collapse, resize, back/forward, New Chat,
   Search, Automations, Customize/Settings, Needs-you → Work, Memory,
   Projects, fullscreen `⌥⌘F`, light and dark, wallpaper tint on the rail.
4. Manual QA in Vite: same layout, fully opaque, no drag (expected).

---

## Rollback

Each phase is one PR / one merge commit on the integration branch.
`git revert` the merge. Phase 6 is the only deletion; reverting it restores
the mock file.

## Definition of done

- [ ] `CursorSidebarMock.tsx` deleted; no `SHOW_CURSOR_SIDEBAR_MOCK` in `src/`
- [ ] `BridgeSidebar` matches the mock's chrome, action rows, grouped list, footer
- [ ] Real behaviors preserved (sessions, filter, scope, Needs-you, Memory,
      Projects, collapse/resize/drawer, drag, nav history)
- [ ] Every phase PR targeted `feat/cursor-sidebar-dev`, never `main`
- [ ] Phase 7: `bun run test` and `bun run build` green
