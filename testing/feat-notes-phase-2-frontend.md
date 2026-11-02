# feat/notes-phase-2-frontend — Test Contract

Issue: #880 (parent #878, epic #865). Backend wire contract already landed (#881: `NotePage.backlinks`, `UpdateNoteParams.parentId` / `clearParent`).

## Functional Behavior

Concrete inputs to expected outputs:

- `[[Page title]]` and `[[Page title|Alias]]` inside a body parse to one `wikiLink` inline node and round-trip byte-for-byte; untouched blocks keep their exact bytes (`keepUntouched`).
- Typing `[[` in `NoteEditor` opens a link menu listing existing pages filtered by the query after `[[`; ArrowUp / ArrowDown move the highlight, Enter inserts a `wikiLink` node, Escape closes. When no page title matches exactly, a `Create "<query>"` option appears; choosing it creates the page (`notes/create_page`) and inserts `[[<query>]]`.
- Clicking a rendered `[[Page title]]` pill calls `pages.openByTitle(target)`: opens the matching page if one exists, otherwise creates and opens it.
- Opening a page that other pages reference (via `[[Title]]` or `<page url="bridge://p/id">`) renders `<section aria-label="Backlinks">` below the child Pages and Tasks sections, one row per linking page (title, kind badge, excerpt quote); clicking a row opens that page.
- Renaming a page in `TitleField` rewrites `[[Old]]`, `[[Old|alias]]` and `<page url="bridge://p/id">Old</page>` blocks in every other note (mock rule mirrors `rewrite_links_on_rename`): the documented core semantics.
- Notes list, in the All and Pages filters, renders child pages nested under their parent with an expand/collapse chevron (`parentId === note.id && kind !== "task"`), recursively indented, expanded set persisted in `localStorage["bridge.notes.expandedPages"]`.
- Each page's More menu offers `Promote to page` when `state === "inbox"` (sets `state: "open"`) and `Move to…` with a picker of other pages (self and descendants hidden) plus a top-level option (`clearParent: true`); the mock rejects cycles.
- `NotePageView` breadcrumb shows the full ancestor chain (`Root > Parent > Current`), each ancestor clickable, not only the immediate parent.
- Selecting or hovering an image in `NoteEditor` shows left/right drag handles; dragging sets the width, clamped 120px to 100%, stored in the Markdown title attribute (`![caption](assets/hash.png "420")`). Untouched images serialize byte-for-byte as plain `![alt](assets/...)`. An editable caption is bound to the image's alt text.

State transitions:
- An inbox note that gets a parent (`parentId`) via Move to… opens (`state: "open"`) in the mock.
- A wiki link inserted from the suggestion menu immediately renders as a pill.

Edge cases (from the spec):
- `[[Old Setup]]` where `Old Setup` only matches with different case → still resolved.
- Rename to a title that wiki links cannot hold (contains `[`, `]`, `|`, newline) → the source's links become `<page url="bridge://p/id">` tags keeping the alias.
- Moving a page under its own descendant → rejected by the mock with an ancestor error.
- Alias-only differences: `[[Target|alias]]` keeps the alias across renames.

## Unit Tests

- `src/components/notes/editor/NoteEditor.test.tsx`
  - `[[Page title]]` and `[[Page title|Alias]]` round-trip and keep untouched blocks byte-exact.
  - Typing `[[` opens the link menu, filters pages, and inserts a wikiLink node on Enter; Escape closes; Create option appears when nothing matches and create + insert on Enter.
  - Clicking a wikiLink pill calls `openByTitle`.
  - Image caption text and drag-width land only in that image block's Markdown (`![caption](assets/… "420")`).
- `src/notesMock.test.ts`
  - backlinks from `[[Title]]` and `<page>` references, excerpt, ordering; none when nothing links.
  - rename propagation rewrites wiki and page tags, keeps aliases, duffcase-safe, falls back to page tag for unspeakable titles, code spans untouched.
  - `parentId` / `clearParent` cycle rejection, tasks-can't-be-parents, inbox promotion.

## Integration / Functional Tests

- `src/components/NotesScreen.test.tsx`
  - Backlinks section lists pages that link to the open page via `[[...]]`, and clicking a backlink opens that page.
  - Renaming a page updates `[[Old]]` to `[[New]]` in the linking page.
  - Nested pages expand/collapse in the Notes list tree; `Move to…` reparents a page and prevents cycles; `Promote to page` opens an inbox note.

## Smoke Tests

- `bun run build` green (tsc + vite).
- `bun run test` green (vitest). Full suite.

## E2E Tests

N/A for this change: the live-app smoke passes belong to phase work that #876 established; this branch runs the vitest suite plus build. WKWebView typing-latency check is a manual step below.

## Manual / cURL Tests

N/A (no HTTP surface; the wire contract is already covered by Rust tests). Manual: open a 200-block page and confirm typing feels as smooth as before; global check that `bun run dev` renders the tree and menus (covered in the live-app check the reviewer can run with `bun run tauri dev`).
