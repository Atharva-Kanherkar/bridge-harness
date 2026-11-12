# feat/notes-editor: Test Contract

The phase 2 editor slice of Bridge Notes, plus two fixes from using phase 1: Structure and Turn into tasks stream while they run, and the notes list collapses. Locked before implementation.

## Decisions
- **Editor: Tiptap 3 (MIT, headless) with `@tiptap/markdown`, not BlockNote.** BlockNote is built on Tiptap, but its Markdown conversion is lossy by its own API name (`blocksToMarkdownLossy`): opening a page and typing one letter would rewrite every other block and drop `<page>` tags. Tiptap lets every block type own its Markdown form, and is unstyled, so the editor is Tailwind tokens only.
- **The page body stays Bridge Markdown in `Y.Text("body")`.** No storage migration in this slice: the editor reads Markdown, and on change writes Markdown through the same `edit_page {text, baseSeq}` path, so agent edits, the `.md` file, history and the three-way merge all keep working unchanged.
- **Untouched blocks are kept byte for byte.** The editor's Markdown is matched block by block against the text it was loaded from; every block whose content did not change is written back exactly as it was (spacing, list markers, emphasis style). Only blocks the person changed take the editor's form.
- **Portable forms** for blocks plain Markdown lacks, chosen so the `.md` file still reads well on GitHub, in Obsidian and in VS Code:
  - Callout: a GitHub alert, `> [!NOTE]` then `> text` (types NOTE, TIP, IMPORTANT, WARNING, CAUTION).
  - Toggle: `<details><summary>Title</summary>` … `</details>`.
  - Math: `$x^2$` inline; a block is `$$`, the TeX, `$$`, each on its own line.
  - Image: `![alt](assets/<hash>.<ext>)` for an uploaded file, or any `https:` URL.
  - Child page: `<page url="bridge://p/<id>">Title</page>` (the tag phase 1 reserved and guards).

## Functional Behavior

### Streaming passes
- `title_jobs_live::one_hidden_turn` takes an optional `on_text` callback, called with the assistant text so far each time it grows. Title jobs pass none.
- `ModelBackend::generate` takes `on_text`. `drain_once` forwards it as `notes/pass_progress {jobId, pageId, kind, text}` (a new Transient notification), at most every 150 ms plus once at the end. Nothing is stored; the settled job and its view stay the truth.
- The page shows the pass as it streams:
  - Structure: the Structured tab fills in the title, kind, tags, summary and each item as soon as each is readable from the partial reply (`partialPass` reads incomplete JSON).
  - Turn into tasks: drafted task titles appear under Tasks as they arrive, marked as drafts, until the pass settles.
- While a pass is queued with no text yet, the Structured tab says "Reading your words…", not the opt-in explainer (the bug in the screenshot).
- A progress event for another page, or for a job that already settled, changes nothing.

### Collapsible list
- The notes list collapses and expands from a button in its header and one in the page header (`PanelLeft`), and with ⌘\ (`toggle-notes-list`, Notes view only, works while typing in the editor). The choice persists in `localStorage` (`bridge.notes.listOpen`). Collapsed, the page takes the full width.

### Editor
- Your words is always editable, like a Notion page: no click-to-edit, no Markdown source on screen.
- Blocks: text, Heading 1/2/3, bulleted list, numbered list, to-do list, toggle, quote, callout, divider, code block (with language), table, image, math block, inline math, child page.
- Inline: bold, italic, strikethrough, inline code, link, inline math.
- `/` opens a block menu at the cursor (drawn in the document body, so a transformed pane cannot offset it) with all blocks above, filtered as you type, arrow keys and Enter to choose, Escape to close. Choosing replaces the `/query` text.
- Markdown shortcuts as you type: `# `, `## `, `### `, `- `, `1. `, `[] `, `> `, ` ``` `, `---`, and `$$x$$` for inline math. `/` opens only at the start of a line or after a space, so a path like `a/b` stays text.
- Selecting text shows a toolbar: bold, italic, strikethrough, code, link, inline math, and Turn into (text, headings, lists, to-do, quote).
- Each block has a handle on hover: drag to move the block; `+` inserts a block below and opens the `/` menu.
- Images: the `/` menu's Image opens a file picker; pasting or dropping an image file also inserts one. The file uploads in parts through `notes/put_asset_part` and `notes/finish_asset`, and the image shows from `notes/read_asset`. Unsupported files and files over 20 MB are refused with a visible reason. A failed upload removes the placeholder and says why.
- Page block: the `/` menu's Page creates a child page (`notes/create_page {parentId}`), inserts its `<page>` tag, and opens nothing; clicking the block opens the page.
- A page whose body starts with `# <its title>` does not show the title twice; the heading is kept in the file.
- Saving: the editor hands Markdown to the draft at most every 250 ms while typing, and at once before a save is forced (Structure, Turn into tasks, Start, restore, leaving the page). Autosave, status and merging are unchanged from phase 1.
- A change from outside (an agent, the `.md` file, a restore) replaces the content when nothing is pending; with pending typing it is merged three ways with `rebaseDraft`, so neither side is lost.
- Enter in the title moves the cursor into the page.

### Assets (backend)
- `notes.db` schema v2 adds `assets(hash, space_id, mime, ext, bytes, size, created_at)`. A blob is named by the first 16 hex characters of its SHA-256.
- `notes/put_asset_part {uploadId?, data}` appends base64 `data` to an upload (new when `uploadId` is absent) and returns `{uploadId, size}`. Parts stay under the 1 MiB request frame. An upload over 20 MiB is refused; uploads not finished within 10 minutes are dropped.
- `notes/finish_asset {uploadId, mime}` stores the blob (deduplicated by hash), writes `<notes_root>/assets/<hash>.<ext>` so the `.md` file's image works in any editor, and returns `{path, mime, size}` with `path` = `assets/<hash>.<ext>`. Accepted: png, jpeg, gif, webp, svg+xml; anything else is invalid params.
- `notes/read_asset {path}` returns `{path, mime, data}` (base64). A path that is not `assets/<hash>.<ext>` or not stored is invalid params.
- Projecting a page whose body references `assets/...` writes any missing asset file into the notes root (so changing the root carries images along).

## Unit Tests
- `bridge-core` notes:
  - `assets_round_trip_by_hash_and_dedupe` (parts, finish, read; the same bytes twice give one row).
  - `asset_uploads_refuse_unknown_types_oversize_and_bad_paths`.
  - `finishing_an_asset_writes_the_file_and_projection_restores_it`.
  - `drain_once_streams_progress_then_settles` (the fake backend calls `on_text` three times; the listener sees the texts in order with the job id and kind).
- `title_jobs_live`: `TurnText` reports growing text through `on_text` and not after the end.
- `bridge-protocol`: registry, params naming, strictness and artifact drift gates pass with the three methods and the notification.
- `src/notesBlocks.test.ts`:
  - splitting a document into blocks loses nothing (`join(blocks) === source`);
  - an unchanged document is returned byte for byte;
  - editing one block keeps every other block's original bytes (odd spacing, `*` bullets, `__bold__`);
  - inserting, deleting and moving blocks keeps the untouched ones;
  - the leading `# Title` heading is split off and put back.
- `src/notesPass.test.ts`: `partialPass` reads title, summary and complete items from a cut-off Structure reply, tasks from a cut-off tasks reply, and returns nothing useful for garbage.
- `src/components/notes/NoteEditor.test.tsx` (jsdom, real Tiptap):
  - each block form round-trips through the editor unchanged: headings, lists, to-dos, quote, callout, toggle, code, table, divider, image, inline and block math, `<page>`;
  - typing in one paragraph of a 200-block page changes only that block in the emitted Markdown;
  - `/` menu filters, Enter inserts a Heading 2, Escape closes;
  - an outside change with no pending typing replaces the content; with pending typing both survive.

## Integration / Functional Tests
- `bridged/tests/notes.rs`: put_asset_part ×2, finish_asset, read_asset over the daemon socket.
- `src-tauri` command signature gates cover the three commands.
- `src/components/NotesScreen.test.tsx` (jsdom, mock):
  - the list collapses and expands from the button, persists, and ⌘\ toggles it;
  - Structure streams: mock progress events fill the Structured tab before the job settles; the queued state says "Reading your words…";
  - Turn into tasks streams draft task titles;
  - typing in the editor autosaves the Markdown; inserting an image through a mocked upload shows it and saves `![…](assets/…)`.

## Smoke Tests
- `bun run build`, `bun run test` green.

## E2E Tests
- Release bundle, real `bridged`: build a page with every block from the `/` menu, paste an image, write math; open the `.md` file and check it reads as Markdown with the image path; reopen the page and check nothing moved. Run Structure on Claude and watch it stream. Screenshots (dark and light) in the PR.

## Manual / cURL Tests
- `bridge exec --json --method notes/put_asset_part --params '{"data":"<b64>"}'` then `notes/finish_asset` returns an `assets/` path; `notes/read_asset` returns the same bytes.
