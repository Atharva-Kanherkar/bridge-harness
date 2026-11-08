# feat/notes-chat-links: Test Contract

When an agent writes or finds a page in Bridge Notes, its reply names the page and nothing more. To
see the page, the person leaves the chat, opens Notes and looks for it. This branch gives every page
a link the agent can put in its reply, and a click on that link opens the page in Bridge.

## Functional Behavior

### The link (`bridge://p/<id>`)
- A page's link is `bridge://p/<id>`, the form the editor already writes for a child page
  (`<page url="bridge://p/<id>">`). `<id>` is the page's id: a UUID from the core, and `note-…` in
  the mock store.
- `src/noteLinks.ts::parseNoteLink(url)` returns the id for exactly that form, case-insensitive on
  the scheme, and `undefined` for anything else. An id is 1–64 characters from `[A-Za-z0-9_-]`. A
  trailing path, a query, a fragment, any other character, and `bridge://x/<id>` all give
  `undefined`. The core decides whether the page exists; a missing one shows the read error.

### Agent tools (`notes/tool.rs`, `mcp_apps/notes_tools.rs`, `notes/agent.rs`)
- Every page summary a notes tool returns carries `"link": "bridge://p/<id>"`. This covers
  `notes_search` (both forms), `notes_read`, `notes_create`, `notes_edit`, `notes_append` and
  `notes_set`.
- The tool instructions and the `notes_create` description tell the agent to put the link in its
  reply as `[Title](link)`, whenever it names a page it made, changed or found.
- The notes agent prompt says the same thing.

### Chat rendering (`Markdown.tsx`)
- `[Title](bridge://p/<id>)` renders as an anchor with that href.
- Any other non-`http(s)`/`mailto` link still renders as plain text. That includes
  `bridge://p/not-an-id` and `javascript:`.

### Opening the link
- The document-wide click interceptor (`externalLinks.ts`) handles a note link itself:
  - it prevents the default action;
  - it hands the URL to the internal router;
  - it never sends it to the system browser or `window.open`.
- App's router takes a note link: it switches to Notes and opens that page. GitHub links route as
  before.
- `NotesScreen` takes an `openRequest` (`{ id, nonce }`). Each new nonce opens that page, including
  the same id twice in a row.
- A link to a page that no longer exists shows the read error, as opening any page does.

## Unit Tests
- `src/noteLinks.test.ts`:
  - `parses a page link`
  - `rejects anything that is not exactly a page link`
- `src/externalLinks.test.ts`:
  - `routes a note link inside the app and never opens a browser`
- `src/components/Markdown.test.tsx` (or the existing Markdown test file):
  - `renders a note link as an anchor`
  - `keeps a bridge link that is not a page as text`
- Rust `notes::tool::tests`:
  - `every page summary carries its bridge link`
- Rust `mcp_apps::notes_tools`: the instructions mention `bridge://p/` (asserted in an existing or a
  new test).

## Integration / Functional Tests
- `src/components/NotesScreen` test: `opens the page an open request names`.
- `src/App` test: `a note link in a chat opens that page in Notes`. If App's harness makes this too
  heavy, the router function is unit-tested instead, and the test is named the same.

## Smoke Tests
- `bun run build`, `bunx vitest run`, `cargo test -p bridge-core --lib notes:: mcp_apps::`.

## E2E Tests
- In the web client against a real `bridged`:
  1. Create a page.
  2. Render a chat message with its link and click it.
  3. Notes opens on that page.

## Manual / cURL Tests
- In the desktop app, ask a chat "save a note titled X". The reply links X, and clicking it opens X.
