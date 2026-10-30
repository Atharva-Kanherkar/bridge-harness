# feat/notes-frontend: Test Contract

Phase 1 frontend for Bridge Notes. Locked before implementation; derived from the frontend issue and the epic's phase 1 sections. Consumes the `notes/*` methods and the `notes/page_changed` notification from the backend; adds no wire changes.

## Functional Behavior

### Reaching Notes
- `AppView` gains `"notes"`. The sidebar main navigation has a **Notes** row (`NotebookPen`) between Projects and Terminals; it is active on the Notes view. The chrome title reads "Notes".
- `open-notes` (⌘⇧O, Go menu "Notes") opens Notes. `capture-note` (⌥⌘N, Go menu "Capture a Note", works while typing) opens Notes with the capture box focused. Neither chord collides with an existing keymap entry; `src-tauri/src/menu.rs` mirrors both.

### Capture
- The capture box sits at the top of the list. Enter saves exactly what was typed (no trim, no rewrite; leading spaces, typos and trailing newlines kept); Shift+Enter inserts a newline. The box clears at once, before the round trip resolves; the new note shows at the top of the Inbox immediately (optimistic row, replaced by the saved one).
- A whitespace-only capture does nothing. A failed capture puts the text back in the box and shows the error.
- Capture calls only `notes/capture_note`. No Structure or Tasks request is made.

### List
- Shows top-level pages (no `parentId`), newest first: title (or the first line of the preview), a preview line, kind, tags, a project chip when linked, and "Agent" when an agent wrote it. Filters: Inbox, All, Tasks; a search field filters through `notes/list_notes {query}`.
- "New page" creates an empty page (`notes/create_page`) and opens it.

### Page: Your words
- Opening a page reads `notes/read_page`. The editor shows `markdown` byte for byte.
- Typing marks the page Unsaved; it autosaves 800 ms after the last keystroke, on blur, and when leaving the page, as `notes/edit_page {id, text, baseSeq}` where `baseSeq` is the seq of the version the draft came from. Status reads Unsaved, Saving, or Saved.
- After a save the page is re-read. If nothing was typed during the save, the editor adopts the merged text and its seq. If typing happened during the save, the typing is rebased onto the merged text (`rebaseDraft`); an overlapping rebase keeps the person's text.
- Title, kind, state and project are editable in the page header through `notes/update_note`. The project control lists every project plus "No project"; choosing "No project" sends `clearWorkspace`. Nothing is disabled for lack of a project.
- A page written or last edited by an agent shows "Written by Claude in <chat name>" (harness label from `agent:<harness>:<session>`, chat name from the session list, "a chat" when unknown). A file import shows "Edited outside Bridge".
- Delete asks first; for a page with tasks the confirmation names how many tasks go with it. Delete calls `notes/delete_note` and returns to the list.

### Page: Structured
- A **Your words | Structured** switch. Structured is read-only: title, kind, tags, summary and items, each item with its verbatim quote and a "Split into note" action (`notes/split_item`), which creates a note and says so.
- With no view yet, Structured explains it is opt-in and offers Structure. A stale view (`structured.stale`) says the words changed since and offers Re-structure. Structuring never changes Your words.

### Model passes and the harness picker
- Structure (and Re-structure) and Turn into tasks open a small harness picker listing the available Claude, Codex and OpenCode adapters, preselecting the chat default when it is one of them, else the first available. Run sends `notes/structure_note` or `notes/extract_tasks` with that harness.
- While a job for the open page is pending or running the button shows "Structuring…" or "Finding tasks…" and the list is polled every 1.5 s until no job is pending or running. A failed job shows its `detail` on the page until a newer job of that kind starts.
- With no Claude, Codex or OpenCode adapter available, the picker says "No harness can run this: sign in to Claude, Codex or OpenCode." and offers no Run. Capture, editing, project linking and Start still work.

### Tasks and Start
- Tasks (the page's `tasks`) render under the page with title, quote (`sourceExcerpt`), state control (open, doing, done) and Start; a started task shows "Open chat" (and keeps Start for another chat).
- Start opens a picker: project (preselected from the task's project, else the page's, else "No project"; every project listed; "No project" always present) and harness (every available adapter, preselecting the chat default). Start reads the task page, builds the brief with `taskBrief`, starts the chat through `submitNewChatDraft` (direct target with no project; the chosen workspace otherwise; the chosen harness either way), then sends `notes/update_note {id, addSessionId, state: "doing"}` for an open task (`addSessionId` only for other pages). Notes stays open; the row shows "Open chat", which opens the chat.
- Any page can be started the same way ("Start a chat" in the page header).

### Live updates
- The screen subscribes to `notes/page_changed`. Any change refreshes the list; the open page re-reads. With no unsaved typing the editor adopts the new text and seq. With unsaved typing the draft and its `baseSeq` are kept, a note says the page changed and the typing will merge on save, and the next save merges in the core.

### Notes chat
- A docked `notes` purpose chat beside the page, remembered by id (`bridge.notes.chatId.v1`), started on first ask as a direct chat titled "Notes", built like the Storage copilot. Each message carries the open page as a `notes-context` fence only when it changed since the last message; user bubbles fold the fence into a "Notes page" chip.

### Notes settings
- A settings control in the Notes header toggles "Auto-structure new captures" (`notes/save_notes_settings {autoStructure}`, off by default) and shows the notes folder.

### Mock mode
- Outside Tauri `bridgeApi` serves in-memory notes: capture, create, edit with a seq history and the same merge rule, update, delete, list, read, fake Structure (title from the first line, one item per non-empty line quoting it) and fake tasks (one per line that starts with `- ` or contains "need to" / "todo"), both completing after a short delay and announcing `notes/page_changed`, plus a seeded agent-written page and `mockNotesAgentWrite(pageId, text)` for tests.

## Unit Tests
- `src/notesBrief.test.ts`
  - `taskBrief` carries title, detail, the excerpt as a quote, the parent page title, and the define-done instruction; omits empty parts.
  - `actorLabel` maps `agent:claude:<id>` to "Written by Claude in <chat>", an unknown chat to "a chat", `file-import` to "Edited outside Bridge", `person` and `model:*` to null.
  - `withNotesContext` / `splitNotesContext` round-trip; text with no fence is untouched; `notesContext` clips long bodies.
- `src/notesDraft.test.ts`
  - `rebaseDraft` keeps an edit before, and after, a concurrent change; reports overlap; identity when nothing changed concurrently.
  - `noteDisplayTitle` falls back to the first preview line, then "Untitled".
- `src/notesMock.test.ts`
  - capture keeps exact bytes and lands in the Inbox; a stale `baseSeq` save merges with an agent write; fake Structure quotes the body and goes stale after an edit; fake tasks quote the page; delete takes tasks; `page_changed` fires per write.
- `src/keymap.test.ts` (existing suites cover the new entries: no collisions, accelerators match `menu.rs`).

## Integration / Functional Tests
- `src/components/NotesScreen.test.tsx` (jsdom, mock `bridgeApi`):
  - capture: Enter saves exactly as typed (typos, indentation), the box clears before the save resolves, Shift+Enter is a newline, no structure or tasks call.
  - opening a page shows its markdown byte for byte; typing autosaves with `baseSeq`.
  - Structure through the picker shows the second view with quotes; Your words unchanged; editing marks it stale with Re-structure.
  - an agent write to the open page with no typing reloads the editor; with unsaved typing the draft survives, and saving keeps both.
  - an agent-written page shows "Written by Claude in <chat>" and is editable.
  - Turn into tasks lists tasks with quotes; Start with "No project" calls `onStartChat` with `workspaceId: null` and the brief, then marks the task doing with the session; Start with a project passes that workspace; both rows show "Open chat".
  - the project control lists every project and "No project"; no control is disabled with zero projects.
  - no generation adapter: the picker explains why and has no Run; capture and editing still work.
  - "Split into note" creates a note; Delete confirms with the task count.
  - a failed job shows its detail.
- `src/components/BridgeSidebar.test.tsx`: the Notes row calls `onOpenNotes` and shows active.
- `src/components/Markdown` user bubble: a `notes-context` fence folds into a "Notes page" chip.

## Smoke Tests
- `bun run build` (tsc + vite) green.
- `bun run test` green (vitest, cargo workspace including the menu parity test, sidecars).
- `bun run dev` opens Notes from the sidebar in mock mode with no console errors.

## E2E Tests
- Manual in the desktop app (or mock mode where noted), recorded as screenshots in the PR:
  1. Fresh data dir, UI only: Notes, capture a messy dump, it shows exactly as typed; no model call.
  2. Structure with a picked harness: Structured view appears; Your words unchanged; edit, view goes stale.
  3. An agent-written page appears live and edits freely; unsaved typing survives a concurrent agent write.
  4. Turn into tasks, Start with "No project" opens a direct chat; Start with a project opens a chat there; both show "Open chat".
  5. Every harness signed out (mock with no adapters): Structure and Turn into tasks explain why.

## Manual / cURL Tests
- N/A for cURL: no new wire surface. Screenshots in the PR: dark, light, the two views, Turn into tasks, the Start picker with "No project", an agent-written page.
