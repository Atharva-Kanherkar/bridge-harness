# feat/t3-inspired-chat-ui: Test Contract

Source: issue #1088 (locked). Mockups and cutouts:
`https://gist.githubusercontent.com/Atharva-Kanherkar/b43e04cd761785d98888973193d11674/raw/t3bridge-*`.
T3 reference shots: same gist, `t3ui-*`.

Global rules for every slice:

- No new surfaces. Every change lands in a component that already ships.
- Graphite & Paper tokens only (`src/index.css`); `src/designSystem.test.ts` stays green.
- A class list that mixes a custom size token (`text-ui`, `text-meta`, `text-caption`, `text-message`) with a text color uses `clsx` or a wrapper, never `cn`, because `cn` drops the size.
- Components never compare harness ids to literals (harness-branch gate test).
- Dock chords come from `dockPaneChord()` (`src/dockLayout.ts`). `DOCK_PANES` order is never changed.
- Shortcuts come from `src/keymap.ts` through `chordLabel()`. New chords get a `keymap.ts` entry and a conflict test.
- Copy says chat, worker, orchestrator, dock, pane; never thread or subagent.
- Every slice is reachable from a fresh install through the UI alone (mock mode counts for visuals).

## Functional Behavior

### A. Turn changes card and turn line
- An assistant turn that edited files ends with a card: `N files changed +A −D in this turn`, a folder tree of the edited files with `M` (modified) or `A` (added) and per-file `+/−`, an Expand all control and a Review button.
- Counts come from the turn's own edit tool calls (`toolCallDisplay(item).additions/deletions/path`). A file edited twice in one turn appears once with summed counts.
- A turn with no edits shows no card. A turn still running shows no card until it settles.
- Review opens the Changes dock pane; its chord hint is `dockPaneChord("changes")`.
- Under the turn: time, duration and model as a caption, next to the existing hover actions (copy, remember, fork).

### B. Steer caption and composer hints
- A message steered into a running turn shows its caption above the bubble, right-aligned: `↪ Steer · lands after this step`. A queued message uses the same placement: `Queued · sent when this step finishes`. Existing `deliveryNotes` semantics do not change.
- Composer key hints follow state: idle `↵ Send · ⇧↵ New line`; while a turn runs `↵ Steer · ⌘. Stop` (or `↵ Queue` when the chat input mode is queue), using `chordLabel("interrupt-turn")`.

### C. Sidebar Running section
- A Running group above Projects lists every chat with a turn or a worker in flight, across projects: title, then `spinner + elapsed · current action` (or `N workers working`), then the harness mark.
- Turn-in-flight is decided by the same signal the transcript uses for `turnActive`, not by `status === "working"` alone (provider-up is not a turn).
- Rows stay in their project folders too. The group hides when nothing runs. Elapsed time ticks once a second. Clicking a row opens that chat.

### D. Chat overview in the dock
- A pane section showing, for the selected chat: Where (worktree path and project, Open in editor), Branch (ahead/behind when known), Changes (count, `+/−`, Review), Checks (latest verification result when present), a Commit split button and Open PR where those actions already exist in Bridge, and per-turn history (title, duration, change count).
- Rows hide when their data is absent; nothing shows invented values. No new dock pane id unless no existing pane fits; if one is added it is appended to `DOCK_PANES`.
- Commit and Open PR reuse existing API paths; if none exists for a chat worktree, the button is omitted and the gap is reported rather than faked.

### E. Worker fan-out card and Agents lineage
- When the orchestrator delegates, the transcript shows one fan-out card per delegation: stacked harness marks (max 3, then `+N`), counts by state (working, needs you, done, failed), elapsed time, and a control that opens the Agents pane.
- While waiting on workers: `Waiting on <worker>, <worker> · Stop`. Stop uses the existing stop path.
- The Agents pane (`TasksPane`) shows lineage: the orchestrator row, then indented workers with harness mark, title, worktree, model, state and timer. Existing open/pin behaviour is kept.
- Only the agent tree is used; the conversation tree is never mixed in.

### F. Model picker columns
- The model control opens a three-column picker: harness rail (favorites first, then available harnesses from the existing per-harness tables), a searchable model list with check on current and star-to-favorite, and a Thinking column rendering the user's effort selector style for the selected model. Footer: speed where supported, access mode.
- Keyboard: Tab cycles columns, arrows move, Enter picks, Esc closes and returns focus to the composer.
- Every existing capability (model switch semantics, effort levels, disabled states while switching) is preserved.

### G. ⌘K actions
- With a non-empty query, the launcher adds an Actions section built from `keymap.ts` entries that make sense from the launcher (label + `chordLabel`), with submenus for parameterised commands (open dock pane). Running an action closes the launcher and performs the same handler the chord runs.
- Empty-query layout (Needs you, Apps, My apps, Recent chats) is unchanged.
- Footer teaches `↑↓ Move · ↵ Run · → Open submenu · ⌫ Back · ⌘K Close`.

### H. @ file rows
- File rows show a file glyph, the file name in mono with the matched substring marked, the folder in `text-faint`, and `M`/`A` on the right for files this chat has changed (from the same turn-edit data as Slice A, or the chat's changes when available).
- Footer teaches `↵ Insert · ⇥ Into folder` and the other triggers.
- Existing insertion behaviour and keyboard handling are unchanged.

## Unit Tests

Component tests use `// @vitest-environment jsdom`, `react-dom/client` + `act`. Run with `NODE_OPTIONS=--no-experimental-webstorage`.

- A: `turnChanges` pure helper: sums repeated edits of one path, marks added vs modified, ignores reads; card renders counts and tree; no card for a turn without edits or while live; Review calls the Changes opener.
- B: steer and queued captions render above the bubble; hints switch between idle and running; Stop hint uses `chordLabel("interrupt-turn")`.
- C: running membership helper (turn in flight vs provider-up only, workers in flight); group hidden when empty; row opens chat.
- D: overview rows render only with data; Review opens Changes; Commit/Open PR call the existing handlers.
- E: fan-out counts by state and `+N` overflow; waiting line names live workers; lineage order orchestrator first, depth indentation.
- F: column focus cycling with Tab; Enter picks model; effort column renders the configured selector style; Esc returns focus.
- G: actions built from `keymap.ts` (drift test: every listed action has a handler); submenu navigation; empty-query sections unchanged.
- H: match marking, folder dimming, changed marker; insertion unchanged.

## Integration / Functional Tests

- Existing suites stay green: `AgentConversation*.test.tsx`, `BridgeSidebar*.test.tsx`, `TasksPane.test.tsx`, `ChatModelControl*.test.tsx`, `AppLauncher.test.tsx`, `App*.test.tsx`, `SessionDock*.test.tsx`, `dockLayout.test.ts`, `designSystem.test.ts`, `keymap*.test.ts`, harness-branch gate test.
- If a wire method is added: `cargo test -p bridge-protocol` (artifact drift) and the method checklist gates.

## Smoke Tests

- `bun run build` green.
- `NODE_OPTIONS=--no-experimental-webstorage bunx vitest run` green.
- `bun run dev` mock mode: open the "New chat · working" chat; every slice's surface renders without console errors.

## E2E Tests

N/A for automated E2E. Visual check through mock mode with Playwright screenshots (dark, light, one interaction per slice), compared with the issue cutouts.

## Manual / cURL Tests

- Mock mode at `http://127.0.0.1:<port>/`: Use recommended defaults, open a chat, then:
  - A: a settled turn with edits shows the changes card; Review opens Changes.
  - B: type while a turn runs: the steer caption sits above the bubble; hints show `↵ Steer · ⌘. Stop`.
  - C: the Running group lists the working chats with ticking timers.
  - D: the dock overview shows branch and changes for the chat.
  - E: the orchestrator chat shows the fan-out card; the Agents pane shows lineage.
  - F: the model picker opens with three columns; Tab moves between them.
  - G: ⌘K, type "dock": Actions list with chords; Enter runs one.
  - H: type `@pol` in the composer: rows show name, folder and changed marker.
