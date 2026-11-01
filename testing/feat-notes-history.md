# feat/notes-history: Test Contract

The last phase 1 slice of Bridge Notes: page history with restore, plus the phase 1 acceptance checks that were only covered by mocks, run in the real app. Locked before implementation.

## Functional Behavior

### Page history (backend)
- `notes/page_history {id}` returns the page's versions newest first, at most the newest 200: `seq`, `actor`, `at` (RFC 3339, from the update's HLC), `bodyChanged`, `added` and `removed` (characters, against the version before), and `title` as of that version. The first version counts everything as added. Property-only changes (title, status, tags) are versions with `bodyChanged: false`.
- `notes/read_version {id, seq}` returns that version: `seq`, `actor`, `at`, `title`, `markdown`. An unknown seq is an invalid-params error.
- `notes/restore_version {id, seq}` writes the version's body as a new change through the one write path, as the person (`person`), and returns `NoteEditResult` (`warnings: ["nothing changed"]` when the body already matches). Only the words are restored; title, status, project and tags stay. A restore can remove a `<page>` tag added later: it is an explicit choice. History is append-only, so the restore is itself a version and can be undone by restoring again. Deleted pages and unknown seqs are refused.
- All three are read or written through `NotesService`, never the DB directly from api; params have `deny_unknown_fields` and are named after the method (`PageHistoryParams`, `ReadVersionParams`, `RestoreVersionParams`).

### Page history (frontend)
- The page's More menu has **History**. It opens a dialog in the style of Notion's page history: versions on the right, grouped so a run of edits by the same author within ten minutes is one entry (the newest version of the run); property-only versions are not listed. Each entry: who ("You", "Claude in <chat>", "Edited outside Bridge", "Turn into tasks"), when, and `+added −removed`.
- Selecting an entry shows that version on the left as a line diff against the current page (added lines and removed lines marked, unchanged lines plain), with a toggle to read the version rendered.
- **Restore this version** restores it (after storing anything typed), closes the dialog and the page shows the restored words; the history then lists the restore as "You".
- Mock mode keeps an actor and time per version so `bun run dev` and the tests work.

### Phase 1 acceptance, in the real app
Run against a real `bridged` on a fresh data dir through the browser client (the desktop UI), with real Claude and Codex chats, recording what each check showed:
1. A Full-access chat asked to "research the top three local-first sync libraries and write it up as a note page" creates a page that appears live in an open Notes view, labeled with that chat, with no approval card.
2. That page edits freely; History shows the agent's version and yours, and restoring works.
3. An Ask-mode Claude chat and an Ask-mode Codex chat each show an approval card before a notes write; rejecting writes nothing.
4. The docked Notes chat creates a child page of the open page, and it shows in the parent's Pages section.
5. `/note buy a domain for the side project` from a chat composer lands in the Inbox with no model turn.
6. Start a task with "No project" (direct chat with the brief) and another in a project (chat in that project); both show "Open chat" and the task is `doing`.
7. Structure and Turn into tasks runs leave no sessions in the chat list.
Anything that fails is fixed in this PR with a regression test, or reported as not done.

## Unit Tests
- `bridge-core` notes:
  - `history_lists_body_changes_with_authors_and_counts`: person capture, agent append, title change, file import; newest first, actors, `added`/`removed`, `bodyChanged: false` for the title change.
  - `history_window_is_the_newest_two_hundred`.
  - `read_version_rebuilds_the_page_at_that_seq` and rejects an unknown seq.
  - `restore_writes_the_old_body_as_a_new_person_version`: later agent edit gone from the body, still in history; title/status unchanged; restoring again undoes it; a deleted page is refused; restoring the current body warns "nothing changed".
- `bridge-protocol`: the existing gates (registry 1:1, params naming, strictness, artifact drift) pass with the three methods.
- `src/notesHistory.test.ts`: grouping by author and ten-minute runs, property-only versions dropped, labels per actor, line diff marks.
- `src/notesMock.test.ts`: mock history records actors and restores.

## Integration / Functional Tests
- `bridged/tests/notes.rs`: history, read_version and restore_version over the daemon socket.
- `src-tauri` `notes_commands_take_every_optional_field_as_option` and the bridge-deck signature gate cover the new commands.
- `src/components/NotesScreen.test.tsx` (jsdom, mock): open an agent-written page, edit it, open History: "You" and "Claude in <chat>" entries; select Claude's version: the diff marks the line the person changed; Restore: the page shows the agent's words and History lists the restore as "You"; unsaved typing is stored before the restore.

## Smoke Tests
- `bun run build`, `bun run test` (vitest, cargo workspace, check:visual, sidecars) green.

## E2E Tests
- The seven real-app checks above, with screenshots for History and the Ask-mode approval cards in the PR.

## Manual / cURL Tests
- `bridge exec --json --data-dir <dir> --method notes/page_history --params '{"id":"<page>"}'` lists versions; `notes/restore_version` restores one.

## Real-app results and the fix they call for

Run on a fresh data dir through the browser client with real Claude and Codex chats:

1. Pass: a Full-access Claude chat wrote "Top Local-First Sync Libraries"; it appeared live in an open Notes view after 57 s, written by that chat, with no card.
2. Pass: edited it; History listed "You +31 −0" and "Claude … +1332 −0"; restoring brought the agent's words back.
3. Fail: Ask mode has no working approval path for notes tools.
   - Claude: Ask mode runs Claude Code with `permissionMode: "acceptEdits"` and no permission callback, so every tool that needs permission is refused with "Claude requested permissions to use mcp__bridge__notes_search, but you haven't granted it yet." No card appears, nothing is written, and even the read-only search is refused.
   - Codex: asked for "my Bridge note", Codex chose its own Computer Use connector, and Bridge shows that connector's MCP approval as an unanswerable "Tool input required" card. Bridge Notes was never called; nothing was written.
4. Pass: the docked Notes chat created "Child check" under the open page in 24 s.
5. Pass: `/note buy a domain for the side project` in a chat landed in the Inbox, written by the person, with no model turn.
6. Pass: Start with "No project" made a direct chat, Start with "Offsite demo" made a chat in that project; both tasks are `doing` with "Open chat".
7. Pass: a real Structure pass (31 s, Haiku, $0.018 on `model_jobs`) ran as a hidden connector session; the chat list stayed at 7.

**Fixed here:** in Ask mode the Claude sidecar allows the `bridge` server's read-only tools (`visualize`, `notes_search`, `notes_read`) without a prompt, since Bridge cannot show Claude's prompt and these change nothing. Notes writes stay refused in Ask mode. Test: `sidecar/claude-agent/test/options.mjs` "ask mode runs Bridge's read-only tools".

**Not done (follow-up):** a real approval card for notes writes in Ask mode. Neither harness offers a path Bridge can render today: Claude has none, and Codex's MCP approvals render as unanswerable forms. The epic's original design fits: the core asks the person through Bridge's own approval card before a write from an Ask-mode chat, with the harness allowing the tool.
