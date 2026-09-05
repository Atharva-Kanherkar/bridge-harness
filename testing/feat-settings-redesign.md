# feat/settings-redesign — Test Contract

Locked before implementation. Derived from issue #511 ("Redesign Settings: one
row-and-group system, Phosphor icons, exact harness marks") and its acceptance
comment. Every criterion below maps to a test named here or to a screenshot in
the PR body.

The single rule the whole redesign rests on: **every page is a column of groups,
every group is a card of rows, every row is one label and one control.**

## Functional Behavior

### Chrome and layout

- The Settings screen renders no page header bar. The 208px rail runs full
  height and carries, in order: the title "Settings", a 28px search field, four
  labelled groups, and a footer with "Reset all settings".
- Rail groups and their items, in this exact order:
  - General: Appearance, Permissions, Composer
  - Agents: Presets, Models, Prompts
  - Runtimes: Harnesses
  - Data: Work briefing, Import
- `Section` ids stay stable for existing callers: `permissions` (opened by the
  bypass badge) and `prompts` (opened from usage). `composer` is added.
  `agents` continues to address the Presets page, `work` the Work briefing page.
- Every page body renders in one centered column of `max-w-[720px]` with 30px
  top padding. Switching pages does not move the column, because no page draws
  its own sidebar or its own width.
- Presets, Harnesses, and Prompts are list pages. A row opens a detail page in
  the same column, headed by a breadcrumb whose first crumb returns to the list.
- Typing in the rail search filters rows across all pages by row label and row
  description, and shows which page each surviving row belongs to. Clearing the
  field restores normal navigation.
- "Reset all settings" sits in the rail footer, opens a confirmation that names
  what it deletes (every Bridge, Codex, Claude, and agent override, and custom
  agent presets), and only calls `resetAllConfig` + `resetModelProfiles` when
  confirmed.

### Controls

- No `<input type="checkbox">` and no `<select>` element exists anywhere under
  the Settings screen root. Boolean rows use a `role="switch"` button; choice
  rows use a listbox-style button + popup.
- Switch is 32x18. Selects, fields, and buttons are 28px tall with 12px text and
  8px radius.
- Group cards use `bg-card` + `border-border-card` at 12px radius with
  `overflow-hidden`; rows are separated by a `border-border` hairline, never by
  margin.
- Filled (foreground) buttons appear only for Save and Connect. Install is a
  ghost (bordered) button, Remove is destructive text, Reset is muted text.
- Type sizes used on the Settings screen are limited to 17 (page title), 13 (row
  label), 12 (control), 11.5 (row description), 11 (group label), and 10.5 mono
  (ids, paths, meta).
- Color appears only on harness marks and status pills. All other chrome is
  achromatic in both Graphite and Paper.

### Saving

- A switch or select row persists on change and shows a check plus "Saved" in
  that row for about 1.5 seconds, then returns to its control.
- Editors and text fields do not persist on change. A dirty page shows exactly
  one save bar at the bottom of the column reading "Unsaved changes" with
  Discard and Save. No per-card Save button exists anywhere in Settings.
- Save is disabled while a request is in flight; a failure is reported through
  the existing `onError` prop and leaves the draft dirty.
- Navigating away from a page with unsaved editor text keeps the draft, exactly
  as Prompt Studio does today.

### Pages

- **Appearance**: mode tiles (Match macOS, Paper, Graphite) and shell tiles
  (Solid, Cursor), each with a two-swatch preview and a radio. The selected tile
  is marked by a foreground border. Choosing a tile stamps
  `document.documentElement.dataset.skin` as it does today.
- **Permissions**: group "Provider prompts" with the auto-approve switch,
  "Always asks" with the two surviving gates behind a lock icon, and "Recent
  auto-approvals" with a row per event (mono body, time on the right, or a
  plain "Nothing has been auto-approved yet").
- **Composer**: group "Inline suggestions" with an Enabled switch row and a
  Model select row. Inline suggestions no longer appears on the Models page.
- **Presets**: list of rows (enabled dot, name, "role · harness", Default pill,
  chevron) and a "New preset" header action. Detail page groups: Identity
  (Enabled, Name, Description, Role), Runtime (Harness, Model, Effort), System
  prompt (editor). Header actions cover Make default and Reset/Delete.
- **Models**: one row per profile grouped Orchestration, Workers, Verification.
  A row shows name, "Provider · Model · effort" with the provider's mark, and a
  "Tracks standard" or "Pinned" pill. Expanding the row reveals the six fields
  plus Allow learning as label/control rows. A stale or errored catalog shows a
  Stale pill with Retry. The header carries the version pill and no Save button.
- **Harnesses (list)**: groups Installed, Available, Defaults. A runtime row
  carries its mark, name, "source · version", a status pill (Ready / Working /
  Not installed / Needs repair), an Install button when the runtime is absent,
  and a chevron. Bridge appears under Defaults as "Always on" with no chevron.
- **Harness detail**: breadcrumb "Harnesses / Name"; header with mark, name,
  "source version · path", status pill, and Remove only when `removable` is
  true. Groups: Sessions (Enabled, Default model, Default effort), System prompt
  (editor), Advanced (Executable path or Advanced JSON), plus Providers and
  Visible models for OpenCode only. Repair replaces Install when `repairable`.
- **Prompts**: list page with a target select in the header and a "Sections"
  group of rows (mono id, token estimate, Modified/Deleted pill, unsaved dot);
  Export overrides and Import overrides are header actions. Detail page:
  breadcrumb "Prompts / Target / section", lint warnings above the editor, a
  file bar with an unsaved indicator, then groups History (row per revision with
  Restore) and Compiled preview (Stable prefix, Variable suffix, and Provider
  layers as expandable rows, including the cache-miss note).
- **Work briefing**: groups Suggested work (Enabled, Harness, Model, Effort),
  What it reads (Everything connected switch, then a switch per connector with a
  sign-in note), When it runs (Cadence, Refresh on focus). The same validation
  as today gates Save.
- **Import**: the existing five-stage wizard, restyled into the page header,
  group cards, and the new controls.

### Icons and marks

- The Settings screen imports no symbol from `lucide-react`. Icons come from
  `@phosphor-icons/react` (Regular weight), 14px in the rail, 12px in controls.
- `harnessMarks.tsx` draws the exact vendor marks: OpenAI (Codex) filled
  `currentColor` in foreground and never green, Claude in `text-harness-claude`,
  OpenCode's frame in `text-harness-opencode` with an inner block at 45%, and a
  new Cursor mark with its own five facets. No radial stand-in remains.
- No mark rotates. `.harness-mark-live` animates opacity only; the
  `harness-turn` keyframe is gone from it.
- Every existing `HarnessMark` caller (startup row, sidebar, aside chat, model
  control) still renders a mark plus its label.

### Copy

- No em dash appears in any user-facing string added or touched by this change.
- The rail's shield note and the header subtitle "Providers, models, prompts,
  and agent presets." are removed.

## Unit Tests

New, in `src/components/settings/settingsKit.test.tsx`:

- `SettingsRow renders one label and one control` — a row with a description
  renders label, description, and exactly one control slot.
- `Switch is a role=switch button and reports aria-checked from its prop` — it
  never flips its own state.
- `Switch does not fire while disabled`.
- `Select renders a button with aria-haspopup=listbox and no native select`.
- `SaveBar disables Save while saving and calls onDiscard/onSave`.
- `StatusPill maps each managed-agent state to its own tone`.

New, in `src/components/settings/settingsSearch.test.ts`:

- `matches rows by label` / `matches rows by description` / `is case-insensitive`
- `returns every page's rows when the query is empty`
- `groups results by the page that owns them`

Extended, in `src/components/harnessMarks.test.tsx`:

- `draws the exact vendor marks for claude, codex, opencode, and cursor` — four
  distinct paths, none produced by the old `arms()` generator.
- `renders Codex in currentColor with no harness tint class` — OpenAI is never
  green.
- `tints Claude and OpenCode and gives OpenCode a 45% inner block`.
- `keeps the unknown-harness spinner and the muted tint`.
- `animates only opacity when live` — the rendered class list still carries
  `harness-mark-live`, and `src/index.css` no longer names `harness-turn` in it.

Retained, in `src/components/PermissionsSection.test.tsx`:

- Every existing case, rewritten against the row form: the switch renders from
  stored state, reports the flip without applying it, is disabled while busy,
  names both surviving gates, lists auto-approvals, and says so when there are
  none. `adapterSupportsAgentRole` coverage is unchanged.

## Integration / Functional Tests

In `src/components/SettingsScreen.test.tsx` (extended, not replaced):

- `renders the rail groups and the nine items in order`.
- `opens Permissions from initialSection and Prompts from initialSection` — the
  two entry points `App.tsx` depends on.
- `has no native checkbox or select anywhere in Settings` — walks every page
  (including each detail page) and asserts
  `container.querySelectorAll("select, input[type=checkbox]").length === 0`.
- `imports no lucide symbol on the Settings screen` — a source-level assertion
  over the `src/components/settings/` directory plus `SettingsScreen.tsx`.
- `keeps the column width constant across pages` — every page body carries the
  same `max-w-[720px]` container.
- `filters rows across pages from the rail search`.
- `confirms before resetting all settings and calls both reset RPCs`.
- `keeps an unsaved editor draft when switching pages and back`.

In `src/components/ManagedAgentsPanel.test.tsx` (extended):

- Install / Repair / Remove still appear only when the API says so, now asserted
  against the harness list rows and the harness detail header.
- The remove confirmation still names the payload and focuses "Keep it".

In `src/components/PromptStudio.test.tsx` (extended):

- Every existing capability still passes through the list/detail split: per
  target stacks, save, reset, reset all, restore, export, import (including the
  malformed-file rejection), lint warnings, compiled preview, and the
  cache-miss note.

In `src/components/WorkSettingsSection.test.tsx` and
`src/components/OpenCodeHarnessSettings.test.tsx` (extended):

- The same validation and the same RPC calls as today, driven through switches
  and selects instead of native inputs.

## Smoke Tests

- `bun run build` is green.
- `bun run test` is green (sidecar + vitest + cargo).
- `bun run dev` on mock data: every one of the nine pages renders, and every
  detail page opens and returns via its breadcrumb.

## E2E Tests

- `bun run tauri dev`: open Settings, walk all nine pages, open each detail
  page, flip one switch on each page and confirm the "Saved" check appears and
  the value survives a reload of the screen.
- Install and remove a managed runtime from the Harnesses page; confirm the
  status pill follows the API result rather than the click.
- Edit a prompt section, navigate to another page and back, confirm the draft
  survives, then Save from the bottom bar.

## Manual Tests

- In both Graphite and Paper at 1440 wide, capture every page and every detail
  page. Confirm the content column does not shift between pages.
- Confirm with the browser inspector that the Settings subtree contains zero
  `select` and zero `input[type=checkbox]` elements.
- Confirm the only colored pixels in the Settings subtree belong to harness
  marks and status pills.
- Grep the diff for an em dash in a quoted user-facing string; expect no hit.
