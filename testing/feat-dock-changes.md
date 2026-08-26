# feat/dock-changes — test contract

Locked before implementation. One workstream: the Changes pane grows into its
dock home — review happens beside the conversation while a turn streams, and
what you are reviewing can be quoted into the next message or opened in the
Code pane without leaving either surface. The panel moves out of App.tsx into
its own component; content redesign beyond dock fit stays out of scope, and
so does line-level targeting in the Code pane, which lands with the code
pane's own feature (the handoff here is path-level, extended later).

## Shape of the thing

`src/components/ChangesPanel.tsx` owns the panel: header, totals, the ranked
file list, the inline diff/editor rows. It fills whatever width its host
gives it instead of centering a reading column, and states what it is
diffing: the workspace branch and the exact phrase "uncommitted vs HEAD"
with the changeset's base commit. Stats drift on the workspace prop —
dirty count, additions, deletions — schedules one debounced in-place reload
that keeps scroll, expanded rows, and viewed marks. Two callbacks leave the
panel: `onQuote(path, range?)` references a file or a hunk in the composer,
`onOpenFile(path)` hands a file to the Code pane through the dock. The hunk
range comes from the patch's own `@@` headers. In App, quoting appends a
file mention (plus `lines a-b` for a hunk) and focuses the composer;
opening dispatches the dock to the code pane and passes CodePanel a reveal
request it honours once per nonce.

## 1. The pane fills the dock — `src/components/ChangesPanel.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The panel renders the review it always rendered | "CHANGES" rank line, file-count heading, +/− totals, viewed counter |
| 1.2 | The panel fills its host instead of centering a column | the scroll root carries no `mx-auto` and no `max-w-3xl` |
| 1.3 | The diff basis is stated | the workspace branch is shown, with "uncommitted vs HEAD" and the base commit from the changeset |
| 1.4 | A clean workspace says so | "Workspace is clean" heading and "No uncommitted changes against HEAD." body |

## 2. Live refresh without losing the review — same file

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Stats drift triggers one debounced reload | changing dirty/additions/deletions props → one further workspaceChanges call after the debounce window |
| 2.2 | Reload happens in place | expanded rows stay expanded, viewed marks stay set, the scroll root is the same node with its scrollTop untouched |
| 2.3 | Unchanged stats do not refetch | re-rendering with identical stats schedules nothing |
| 2.4 | A workspace switch still resets the review | new workspace id → collapsed rows, fresh load |

## 3. Quote to composer — same file

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | A file row offers a quote action | a button labelled "Reference src-tauri/bridge-core/src/policy.rs in the composer" calls `onQuote` with the path and no range |
| 3.2 | A hunk offers a quote action with its new-file range | in the expanded diff, the `@@ -10,7 +10,21 @@` hunk yields a button labelled "Reference lines 10-30 in the composer" calling `onQuote` with `{start: 10, end: 30}` |
| 3.3 | No callback, no affordance | without `onQuote` neither button renders |

## 4. Handoff to the Code pane — same file, plus `src/components/CodePanel.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | A file row offers the Code pane | a button labelled "Open src-tauri/bridge-core/src/policy.rs in the Code pane" calls `onOpenFile` with the path |
| 4.2 | CodePanel honours a reveal request | rendering with `reveal={{path, nonce}}` opens that file: its tab appears and is active |
| 4.3 | A reveal fires once per nonce | re-rendering with the same nonce does not re-open; a new nonce for the same path re-activates it |

## 5. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | Quoting a file lands in the composer and focuses it | after the quote action, the composer value contains the file mention and the textarea is the active element |
| 5.2 | Quoting a hunk carries the range | composer value ends with the mention plus "lines 10-30" |
| 5.3 | Opening from the diff switches the dock to Code | the Code tab is selected and the file's tab is active in the pane |
| 5.4 | The switcher badge stays truthful while the pane is hidden | with another pane active, the Changes tab still shows the workspace dirty count |

Explicitly **not** changed: `PatchView`'s rendering of rows (it gains only an
optional per-hunk affordance when a callback is passed); the inline editor
and its dirty-buffer keep-alive; importance ranking and low-signal folding;
the policy engine; every dock shell behaviour locked in
`testing/feat-dock-shell.md`. The design-system guard stays green with no
new allowlist entries.
