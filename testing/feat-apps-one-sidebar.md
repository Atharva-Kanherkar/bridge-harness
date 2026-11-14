# feat/apps-one-sidebar: Test Contract

Slice 1 of the apps epic: one sidebar, a ⌘K launcher, pins, a per-chat dock
and the Store shell. Frontend and the native menu only. No protocol change.

## Functional Behavior

### Sidebar
- On a fresh install (no pins stored) the sidebar shows, top to bottom: New
  Chat, Search, the **Apps** row, the chat list with its Projects header, and
  Settings in the bottom rail. Nothing else in the main nav.
- Removed: the 9 main-nav rows (Marketplace, Workflows, Projects, Notes,
  Terminals, Mission Control, Memory, Usage, Pulls), the Agents header button,
  and the bottom-rail Pulls button. The usage dot (`railTrailing`) stays.
- The Search button still opens the sidebar's own chat search, unchanged.
- The Apps row carries one badge: workflow runs waiting on you plus memory
  proposals. Hidden at zero. When the memory count is capped (`50+`), the
  sum reads `N+`. The accessible name reads "Apps, N waiting for you".
- Pinned apps render as rows under the Apps row, in pin order, with the same
  active highlight the old rows had, and their own badge where they have one.

### Launcher
- Opens from the Apps row, from ⌘K anywhere outside the terminal, and from
  the native menu item "Apps". Opening twice is idempotent. ⌘K while it is
  open closes it.
- Sections, in order, each hidden when empty:
  1. **Needs you**: every app with a non-zero badge.
  2. **Apps**: every built-in app, pins marked.
  3. **Recent chats**: up to 5 top-level chats, most recent first.
  4. **Actions**: "Search this chat" (only while a chat is open), and
     "Search all chats for “q”" (only when the query has at least
     `CHAT_SEARCH_MIN_CHARS` characters).
- Typing filters apps by name and keywords, and chats by title,
  case-insensitively. A query with no match shows "No app or chat matches".
- The first item is highlighted. ArrowDown and ArrowUp move through every
  item across sections and wrap. Enter opens the highlighted item. Clicking
  opens. Esc or a click outside closes. Opening an item closes the launcher.
- ⌘P toggles the pin on the highlighted app. With 3 pins, pinning a 4th shows
  "You can pin 3 apps. Unpin one first." in a `role="status"` line and
  changes nothing.
- "Search all chats for “q”" opens the sidebar's search on `q` and runs the
  deep stage, as `/find` does today.
- "Search this chat" does what `open-recall` does.

### Apps (built-in registry)
- Apps: Notes, Pulls, Workflows, Memory, Mission Control, Terminals, Usage,
  Projects, Agents, Store. Each maps to exactly one `AppView`.
- Every `AppView` is either an app's view or on an explicit non-app list
  (`workspace`, `work`, `settings`, `archives`, `briefing-settings`).
- The active app follows the view: `mission-control` also when the workspace
  is in the grid paradigm.

### Pins
- Stored in localStorage under `bridge.apps.pins.v1`. Unknown ids, duplicates
  and anything past 3 are dropped on read. Unreadable storage reads as no pins.
- Pin order is the order pinned. Unpinning removes it. Every surface (sidebar,
  launcher, Store) updates together without a reload.

### Keymap
- New `open-launcher`: ⌘K, Go menu "Apps", accelerator `CmdOrCtrl+K`.
- `open-recall` moves to ⌘F, accelerator `CmdOrCtrl+F`, label unchanged.
- `src-tauri/src/menu.rs` mirrors both. No chord is bound twice.

### Dock
- Visible panes = (core panes `work`, `changes`, `terminal`, `browser` when
  available) ∪ (any available pane with a badge or alert) ∪ (the active pane)
  ∪ (panes added for this chat). Strip order stays `DOCK_PANES` order.
- A **+** button opens a menu of the hidden panes. Picking one adds it to this
  chat and opens it. The button is absent when nothing is hidden.
- Added panes persist per chat in localStorage (`bridge.dock.added.v1`),
  capped at the 200 most recently touched chats.
- ⌥⌘digit still opens `DOCK_PANES[index]`. A hidden pane opened that way
  shows because it is active.
- The session toolbar is unchanged.

### Store
- The `marketplace` view's title reads "Store".
- Tabs: **Apps** (default), **Plugins** (today's plugin catalog, unchanged),
  **Skills** (unchanged).
- The Apps tab lists every built-in app with its description, the places it
  shows up, Open, and Pin or Unpin. The pin limit message is the launcher's.

### Out of scope
Manifests, core registry, `apps/*` methods, community installs, trails.

## Unit Tests
- `src/apps/registry.test.ts`
  - every `AppView` is an app view or on the non-app list (reachability gate)
  - no two apps share a view; ids are unique
  - `appForView("workspace", "grid")` is mission-control
  - `appsBadge` sums, hides at zero, and reads `N+` when memory is capped
- `src/apps/pins.test.ts`
  - read drops unknown, duplicate and fourth ids
  - pin appends, unpin removes, a 4th pin returns `{ ok: false }`
  - corrupt JSON reads as `[]`
  - subscribers hear every change
- `src/dockVisibility.test.ts`
  - core panes only when available
  - a non-core pane shows with a badge or alert, or when active, or added
  - hidden panes keep `DOCK_PANES` order
  - added-pane storage caps at 200 chats
- `src/keymap.test.ts`
  - ⌘K resolves to `open-launcher`, ⌘F to `open-recall`
  - no two shortcuts share a chord (new test)
  - menu parity (existing test, now covering `open-launcher`)

## Integration / Functional Tests
- `src/components/AppLauncher.test.tsx` (jsdom)
  - sections in order; empty sections hidden
  - typing filters apps and chats; the no-match line
  - ArrowDown, ArrowUp wrap; Enter opens the highlighted app via `onOpenApp`
  - ⌘P pins and unpins; the 4th pin shows the status line
  - "Search all chats" calls `onSearchChats(q)` only past the minimum length
  - "Search this chat" appears only with a chat open
- `src/components/BridgeSidebar*.test.tsx`
  - no main-nav rows; the Apps row and its badge; pins in order with active state
  - no Agents header button, no rail Pulls button
  - the existing search, archive and interaction suites still pass
- `src/components/SessionDock.test.tsx`
  - the + menu lists hidden panes and calls `onAddPane`; absent when none hidden
- `src/components/MarketplaceScreen.test.tsx`
  - tabs read Apps, Plugins, Skills; the plugin suite runs under Plugins
  - the Apps tab pins and unpins

## Smoke Tests
- `bun run build` and `bun run test` green.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-deck menu` green
  (the shell crate's menu tests).

## E2E Tests
- Mock mode (Vite) driven over CDP from a fresh profile, using only the UI:
  open every app from the launcher and confirm the view title; pin 3 apps,
  reload, confirm the pins; try a 4th; open a chat, open the dock, add a pane
  through +, reload, confirm it stays on that chat only.
- Screenshots, dark and light: sidebar, launcher, dock + menu, Store Apps tab.

## Manual / cURL Tests
- N/A for cURL (no network surface).
- Manual in the desktop build: ⌘K from the composer opens the launcher; ⌘F
  opens chat recall; the Go menu shows "Apps ⌘K" and "Search This Chat ⌘F".
