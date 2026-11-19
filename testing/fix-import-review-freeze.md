# fix/import-review-freeze: Test Contract

Importing from Claude Code is one button and a background job. The Review step is removed: on a real Mac with 85,057 Claude transcripts it took 65 s and 5.1 GB of memory and sent a 63 MB list to the window, which froze it.

## Functional Behavior

- Settings has a Chat history entry in its sidebar, before Data & storage. Its page holds Import and Archived chats; each opens with a "Back to Chat history" link, and Data & storage no longer lists them.
- The Import page lists the tools found on this Mac. Claude Code has one button, Import. Nothing else is asked before importing.
- Import starts a job in Bridge's own process and returns at once. The window can leave the page or close; the job keeps going.
- The job imports every real chat and the setup items (instructions, settings, agents, commands, skills, rules). Setup arrives switched off. Background runs (chats Claude Code ran in the OS temp folder) are left out and counted as left out. Claude Code's memory files are not imported: Claude Code in Bridge keeps reading them itself, so copying them would only duplicate them into Bridge memory.
- The job builds its work list from file paths and sizes only: no transcript is opened to list it and nothing is sent to the window. It then reads and imports a small batch at a time in a fixed order, bounded by count and by bytes, so memory stays bounded whatever the number of transcripts.
- Progress (processed of total, came over, already in Bridge, the current project) is saved after every batch and published as a live event. The page shows it as a progress card with a transfer stage, after a file copy on macOS or Windows: the tool's tile, Bridge's tile, and chat cards arcing between them while Bridge's tile pulses as they land. It says how long is left from the pace so far. When the import ends the cards stop, the path turns solid and Bridge's tile shows a check. Under reduced motion no cards are drawn.
- When the job finishes, Bridge tells the person: an in-app toast, plus a system notification when Bridge is not focused. The page shows what came over, what was already in Bridge, the background runs left out, and what did not come over.
- Only one import runs at a time. Pressing Import while one runs shows the running one.
- If Bridge quits mid-import, the job resumes on the next start from the last finished batch. Chats imported earlier are recognised and not imported twice.
- Importing again later brings over only what is new or changed.
- One item Bridge refuses costs only that item: each chat with its subagents, and each other item, commits in its own transaction. The refused item is listed under "did not come over" with the reason, and the rest of its batch lands.
- The Import page is the found tools and an Import button. The folder wizard is gone: its folder picker ran the same full listing that froze the window.

## Unit Tests

- `claude_import`: the job's work list is sorted, skips temp-folder projects, counts them as left out, includes memories and setup, and resumes after a cursor.
- `import_job`: batches respect the count and byte limits; the report adds up imported, already in Bridge, changed, failed and not-carried reasons across batches.
- `moveToBridge.ts`: not-carried keys render as readable text; the time left reads from the pace so far, and waits for a few seconds of pace.

## Integration / Functional Tests

- `import_job` on a real store: a fake Claude home with chats in two projects, a background run, a memory and an agent. The job imports the chats, leaves the background run and the memory where they are, imports the agent switched off, and publishes progress and a finished state.
- Running the job twice imports nothing the second time; everything is already in Bridge.
- A job left `running` (Bridge quit) resumes on startup from its cursor and finishes.
- One unreadable transcript is reported under "did not come over" and the rest import.
- A transcript that vanishes mid-import costs only itself.
- An item Bridge refuses (built with a memory over 4,000 characters) is refused alone and reported; the chats in the same batch import.
- No Claude Code memory file lands in Bridge memory.
- `MoveToBridge.test.tsx`: Import starts the job; a progress event animates the card; the transfer stage draws six cards from Claude Code to Bridge with the time left; a finished event shows the summary and the stage with a check and no cards; reopening the page during a job shows its progress.
- Both hosts (bridged and the embedded desktop) resume an interrupted import at startup.
- `sections.test.ts` and `SettingsScreen.test.tsx`: the rail is General, Coding agents, Permissions, Chat history, Data & storage; Chat history opens Import and Archived chats; Data & storage lists neither.

## Smoke Tests

- `bun run build`
- `bun x vitest run src/components/MoveToBridge.test.tsx src/moveToBridge.test.ts`
- `cargo test -p bridge-core --lib import`
- Protocol and shell contract tests: `cargo test -p bridge-protocol`, `cargo test -p bridge-deck --lib`.

## E2E Tests

- Real store measurement: run the job's work-list and batch path against this Mac's `~/.claude` (aggregate numbers only) and record time and peak memory against the 65 s and 5.1 GB of the old Review.
- Scripted fake user (`scripts/e2e/import-fake-user.mjs`) against a real `bridged` on a fake home: Import, progress, leave the page, finished notice, summary, imported chats, import again.

## Manual Tests

- Click Import, leave the page, come back: the progress card is still moving.
- Wait for the finish: a toast appears, and a system notification when Bridge is in the background.
- Quit Bridge mid-import, reopen: it carries on.
