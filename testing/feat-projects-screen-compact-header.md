# feat/projects-screen-compact-header — Test Contract

Three changes the user asked for after #201 landed: projects become their own
screen instead of a tree in the rail, the duplicated header meta line goes, and the
Agent/Changes/Code/Terminal tab row collapses into a compact icon strip.

## Scope And Stated Assumptions

- **Projects leave the rail entirely.** The rail lists chats; projects get a screen
  reached from the footer beside Marketplace and Settings. The rail keeps its
  `workspaces` prop only because `Group by → Project` needs workspace titles for its
  group labels.
- **The rail is split by scope, not by tree.** A `Work` / `Code` switch sits above
  New chat: Work lists chats with no project, Code lists chats inside one. Both
  halves keep the same day headers, filter menu, search and cap. This replaces the
  tree as the way project work is reached from the rail.
- **A chat's scope follows from its data, not from a setting.** `workspaceId` decides
  it: no project means Work. Nothing else needs storing, and an existing chat cannot
  be in the wrong half. The scope ids are `work` and `code` in the source too, so the
  label and the code cannot drift apart.
- **Every chat still lives in exactly one list.** The tree is gone, so no chat is
  listed twice; `Group by → Project` stays meaningful inside Code.
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

### New chat
- The rail's New chat opens a dialog rather than creating immediately. It asks the
  two questions every chat needs answered: which project, and whether to take an
  isolated worktree.
- `No project` is preselected and produces a plain Home chat through the existing
  `createChat` path.
- Choosing a project reveals the worktree checkbox. It is disabled, with a reason,
  for a workspace with no repository behind it (`projectId` absent) — the same
  condition the orchestrator dialog already checks.
- The confirm names the destination: `Start chat`, or `Start in <project>`.
- Escape closes. Each visit starts from the caller's intent, not the previous
  answer, so a dialog opened from a project card preselects that project.

### Scope switch
- Two tabs, `Work` and `Code`, persisted under `bridge.sidebar.scope`, defaulting to
  Work. Both stay reachable in the collapsed rail as icons. Work carries a
  conversation icon rather than a house — it is the plain-chat half, not a home
  screen.
- Opening a chat switches the rail to that chat's scope, once per chat id — so a new
  plain chat started from Code, or a project chat opened from the projects screen,
  is never created into a list the rail is not showing. A manual switch afterwards
  survives the next poll.
- An empty Code list says where project chats come from rather than just "no chats".
- `Group by → Project` is offered in Code only. Nothing in Work has a project, so
  grouping by one there would produce a single `No project` bucket and call it a
  grouping. A project grouping carried over from Code is corrected to `Date` and the
  correction is persisted, so the stored view and the rendered list never disagree.

### Group folding
- Every group header is a button that folds its own group away, with `aria-expanded`
  and a chevron. The header and its count stay visible while folded, so there is
  something to unfold from.
- Folds are keyed by group key and reset when the grouping or the scope changes,
  since those re-key every group.
- The collapsed rail does not fold, for the same reason it does not cap: no header,
  nowhere to unfold from.

### Session toolbar
- One row replaces the old title block and the tab strip: title, segmented tab
  control, quiet context text, overflow menu.
- Tabs are icon-only except the active one, which also shows its label. Every tab
  keeps `title` and `aria-label`, and the active one carries `aria-pressed="true"`.
- `Changes` shows its dirty-file count whether or not it is the active tab.
- The segmented control is absent when the session has no repo — a direct chat has
  one panel, so a one-item control would be furniture.
- Context text is the model alone, hidden below `lg`. The branch and dirty count are
  deliberately absent: the count already rides on the Changes tab, and the branch
  name in a header is the noise this strip exists to remove.
- The overflow menu holds Browser (checked while open), Learning router settings
  (repo sessions only), Fullscreen, and End chat (only while the session is live).
  End is styled destructive and disabled while a turn is in flight.
- In fullscreen the row is the topmost strip, so it keeps the left inset that leaves
  the traffic lights their corner and stays a drag region.

### Rail
- No projects tree, no `New project` button in the rail, no per-workspace rows in
  the collapsed rail.
- Order: brand row, scope switch, New chat, search, history, footer, with 12px
  between the top controls and 16px under New chat, so the action is not crowded
  against either the switch or the list.
- Footer order: Projects, Marketplace, Settings.
- Everything else from #201 is unchanged: day headers, filter/grouping popover,
  search, the 12-row cap, collapse, resize, the below-`sm` drawer.

### Window controls (macOS)
- The three traffic lights are hidden at startup and revealed while the pointer is
  within 112×44 of the top-left corner — the region `trafficLightPosition (18,15)`
  plus the cluster's own width, with slack.
- The reveal is driven from the web layer, the only side that sees the cursor, and
  reaches AppKit through one local command. That command is **not** a
  `bridge-protocol` method: window chrome is not something a remote daemon can
  serve, so it is routed ahead of the protocol lookup and stays out of
  `generate_handler![...]`, leaving the 1:1 registry contract untouched.
- Losing focus or leaving the window hides them again.
- Non-macOS builds compile to a no-op; no other platform draws its controls over
  the client area.

### Chat titles
Every session read `Orchestrator`, because that is the label an orchestrator is
created with and nothing replaced it. A title now resolves in preference order:

1. **the harness's own**, where it keeps one. Claude Code writes
   `{"type":"custom-title","customTitle":…}` into
   `~/.claude/projects/<project>/<provider-session-id>.jsonl` once it has seen
   enough to name the conversation; the last such entry wins, since a chat can be
   renamed. Codex keeps no title — a full rollout file carries `session_meta`,
   `event_msg`, `response_item`, `world_state` and `turn_context`, none of which
   names the conversation. OpenCode keeps one behind its HTTP API, which this change
   does **not** read yet (see below).
2. **a heading cut from the first substantive user message** — first meaningful
   line, markdown furniture stripped, whitespace collapsed, shortened on a sentence
   or word boundary at 60 characters, first letter capitalised unless it is a URL.

Low-signal openers never name a chat: greetings, a message under six characters, a
pleasantry opener in a message of fewer than four words (`hey man`), a redacted
`[secret:…]` placeholder, or a bare `@mention`. A chat that only ever greets keeps
its placeholder, because a rail full of `Hello` is no better than one full of
`Orchestrator`. Terse instructions (`fix migration`) are kept.

Provenance decides what may be replaced. `sessions.title_source` (migration 23)
records whether a title was `derived` by Bridge or read from the `provider`; an
absent source means it predates the column and belongs to the user. Only a
`derived` title is ever replaced, and only by a provider title — so the heading cut
from the first message is a stand-in that Claude's own title supersedes when it
appears, and a title the user or the harness chose is final.

The provider read never runs under the database lock. `plan` reads what is needed in
one cheap pass, `resolve` does the directory walk and transcript read with no
database handle at all (enforced by its signature), and `commit` writes the result.
The live-turn caller runs the three phases so the process-wide lock is free while
the filesystem is read.

Timing:
- **On each completed turn**, until the session has a provider or user title. Claude needs a
  turn or two to write its own, and a session has nothing to be named after until
  it has said something.
- **Once at database open**, as a catch-up for chats that predate titles. This pass
  is local-only — no file or network I/O — so opening the database stays cheap; a
  provider title is picked up on that session's next completed turn.

Deliberately out of scope, and why: OpenCode's own title is not read back. The
adapter pins `"title": "Bridge session"` at creation, so reading it back today would
return that constant, and changing the create call is a live protocol change this
branch cannot exercise. OpenCode sessions are titled from their first message like
Codex ones, and the read-back is a follow-up.

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
- Offers Work and Code, Work selected by default, and honours a persisted scope.
- A project chat is absent from Work and present under Code.
- An empty Code list explains where project chats come from.
- The switch stays present in the collapsed rail.
- Existing cases stay green, including `Group by → Project` labels, which still
  need the `workspaces` prop.

`src/components/BridgeSidebar.interaction.test.tsx` — new, jsdom, for behaviour the
static suite cannot reach:
- Folding a group hides its chats and keeps its header, count and `aria-expanded`.
- A fold does not survive a scope change, which re-keys every group.
- Switching scope swaps which chats are listed and persists the choice.
- Opening a chat follows the rail to that chat's scope, in both directions.
- A manual switch survives a re-render with the same active chat.

`src/components/NewChatDialog.test.tsx` — new:
- Renders nothing while closed.
- Lists `No project` plus every project, with `No project` chosen by default.
- Reports `{workspaceId: null, worktree: false}` for a plain chat.
- Reveals the worktree checkbox only once a project is chosen, and reports the
  project and worktree together.
- Disables the worktree option, with a reason, for a workspace with no repository,
  and still reports `worktree: false` for it.
- Names the project on the confirm button.
- Preselects `initialWorkspaceId`, and forgets the previous answer between visits.
- Closes on Escape; holds the confirm while busy.

`src-tauri/bridge-core/src/session_titles.rs` — new, 21 unit tests:
- `needs_title` treats every creation label and blank as unnamed.
- A heading is the first meaningful line, capitalised, with markdown furniture and
  quoting stripped and whitespace collapsed.
- A long message is cut on a sentence boundary when there is one, else on a word
  boundary, never mid-word; a URL keeps its lowercase scheme.
- Greetings, sub-six-character fragments, `hey man`-shaped pleasantries, redacted
  secrets and bare `@mentions` are low signal; terse instructions are not.
- A chat opening with a greeting is named by its next message; a chat that only ever
  greets stays unnamed.
- Claude's title is read from a transcript, the last entry wins, a `summary` entry
  serves an older transcript, a transcript with no title yields none, and a
  transcript is found by session id under any project directory.
- `refresh` titles a Codex session from its first message, leaves a real title
  alone, no-ops for a silent or unknown session.
- The backfill names old chats, skips named and silent ones, and renames nothing on
  a second run.
- `real_claude_transcripts_yield_titles` (`#[ignore]`) parses the developer's actual
  `~/.claude` directory. It is what proves the parser matches what Claude writes
  rather than what this file assumes: 101 titles across 7850 transcripts.

`src/trafficLights.test.ts` — new:
- The reveal region covers the buttons and stops short of the rail's own controls.
- The watcher is inert outside the desktop shell, where there is no chrome to hide.

`src-tauri/src/window_chrome.rs` — new unit test:
- The chrome command is not a `bridge-protocol` method, so the 1:1 registry test
  cannot start demanding it appear in `generate_handler![...]`.

`src/components/sidebarChats.test.ts` — extended:
- `chatScope` maps a workspace to Code and its absence to Home.
- `inScope` splits a mixed list without dropping anything.
- The scope round-trips through storage and falls back to Home on a bad value.

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

0. New chat opens the dialog; picking `No project` produces a Home chat, picking a
   project produces a Code chat and offers a worktree.
0b. Fold a day header: its chats hide, its header and count stay, the chevron turns.
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
