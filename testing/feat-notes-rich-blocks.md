# feat/notes-rich-blocks: Test Contract

Chat already draws a fenced block as a Mermaid chart, a Bridge diagram, math or sandboxed HTML. A
note shows the same fence as plain code, even when an agent wrote the note in that chat. This branch
reuses the chat's renderers inside the page editor, so a fence looks the same in both places. The
`.md` file stays an ordinary fenced block that GitHub, Obsidian and VS Code read.

## Functional Behavior

### One renderer (`Markdown.tsx`)
- `richFenceKind(lang)` names the rich kind of an info string: `mermaid`, `diagram`, `html`, or
  `math` (`math`, `latex`, `tex`). It is case-insensitive and trims whitespace. Any other info
  string gives `null`. It uses the same table as the chat's `splitBlocks`, so the two cannot
  drift.
- `RichFence({ lang, body })` renders that block with the chat's own `MermaidBlock`,
  `DiagramBlock`, `HtmlBlock` or `MathBlock`, and renders nothing for any other info string.
- The chat's output is unchanged.

### In the page editor (`notes/editor`)
- The code block is `@tiptap/extension-code-block`, as before (StarterKit's, now loaded
  directly). Markdown in and out is unchanged: a ```` ```mermaid ```` fence round-trips byte for byte.
- `NoteCodeBlock` (in `views.tsx`) is the editor's code block with a React view:
  - **A rich fence with a body, the cursor outside it:** the rendered figure shows. The source
    `<pre>` stays in the DOM but is hidden.
  - **The cursor inside it (editor focused):** the source shows for editing, and the figure is
    hidden.
  - **Clicking the figure:** puts the cursor at the end of the source. A click on one of the
    figure's own buttons (copy, expand) does not.
  - **An empty rich fence, or any other language:** a plain code block, as now.
- `noteExtensions` takes a `codeBlock` option, as it does for `image` and `pageLink`. Tests that
  build the schema without React get the plain block.
- Mermaid keeps `securityLevel: "strict"`, and HTML keeps the `sandbox=""` iframe: the same
  trust boundary as in chat.

### Slash menu
- A "Mermaid diagram" item inserts a code block with language `mermaid`.

## Unit Tests
- `Markdown.test.tsx`:
  - `names the rich fence kinds and nothing else`
  - `renders a rich fence with the chat's renderer`
- `NoteEditor.test.tsx` (or a new `codeBlock.test.tsx`):
  - `round-trips a mermaid fence unchanged`
  - `shows the figure for a rich fence and hides its source`
  - `shows the source while the cursor is inside`
  - `clicking the figure moves the cursor into the source`
  - `leaves an ordinary code block as code`
  - `the slash menu inserts a mermaid block`

## Integration / Functional Tests
- The existing `NoteEditor` and `NotesScreen` suites pass unchanged.

## Smoke Tests
- `bun run build`, and `bunx vitest run` across the whole frontend.

## E2E Tests
- Mock-mode screenshots, dark and light, of a page with a Mermaid fence, a diagram fence and a
  math fence, both as figures and with the cursor in one.

## Manual / cURL Tests
- In the app, type ```` ```mermaid ```` then `graph TD; A-->B`, and click out. The chart draws.
  Click it, and the source is back.
