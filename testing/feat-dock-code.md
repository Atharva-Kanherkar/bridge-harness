# feat/dock-code — test contract

Locked before implementation. One workstream: the Code pane opens from
anywhere the conversation names a file — a tool call's path, a path in
assistant prose, a mention in a user message — at the right line where a
line is known, and it reflects the agent's writes to files that are open,
without ever clobbering an unsaved buffer. The save-time conflict flow
(refused write, Reload/Overwrite in the status bar) already exists and is
not redesigned; this feature makes the same conflict visible before a save
is attempted.

## Shape of the thing

The reveal request grows a line: `reveal={{path, line?, nonce}}`. CodePanel
passes the line to the editor once the revealed file is active, and
CodeEditor moves its cursor to that line and scrolls it into view, one move
per nonce. Workspace stats drift — the same signal the Changes pane rides —
makes CodePanel re-read every open file's hash: a clean buffer whose disk
bytes moved reloads in place, a dirty one flips to the existing conflict
state with the existing affordances, and its text is untouched. In the
conversation, AgentConversation and Markdown accept `onOpenFile(path,
line?)` plus the workspace file list: an edit or read tool row's path
becomes a button, an inline code span that resolves to a workspace file
(with an optional :line suffix) becomes a button, and an @mention in plain
message text becomes a button. Paths that do not resolve stay plain text —
a dead link is worse than none.

## 1. Reveal at a line — `src/components/CodePanel.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | A reveal with a line reaches the editor once the file is active | the (mocked) editor receives `revealLine` with that line and the reveal nonce |
| 1.2 | A reveal without a line passes none | `revealLine` stays absent |
| 1.3 | The line does not leak to another file | after switching to a different tab, its editor carries no `revealLine` |

## 2. Disk drift on open buffers — same file

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | An agent edit to a clean open buffer is reflected | drift signal + changed sha → the buffer reloads: editor reseeded with the new content |
| 2.2 | An agent edit to a dirty open buffer prompts, never clobbers | drift signal + changed sha → state "conflict" with "changed on disk" in its message, the editor keeps the unsaved text, Reload and Overwrite are offered |
| 2.3 | An unchanged file does nothing | drift signal + same sha → no reload, no conflict |
| 2.4 | No open files, no reads | drift with nothing open issues no readWorkspaceFile calls |

## 3. Links out of the conversation — `src/components/AgentConversation.test.tsx` and `src/components/Markdown.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | An edit/read tool row's path is a button when the callback exists | clicking it calls `onOpenFile` with the tool's path |
| 3.2 | Without the callback the path stays inert | no button, the detail renders as before |
| 3.3 | Inline code naming a workspace file opens it | `src/App.tsx:42` in a code span → button firing `onOpenFile("src/App.tsx", 42)`; without the suffix, no line |
| 3.4 | A path that does not resolve stays plain code | `src/nope.ts` renders as a plain code span |
| 3.5 | An @mention in plain message text opens its file | `@src/App.tsx` in a user message → button firing `onOpenFile("src/App.tsx", undefined)` |

## 4. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | A sent mention becomes a live link back into the editor | send a message containing `@src/App.tsx`, click the rendered mention → the Code tab is selected and the file's tab is open |

Explicitly **not** changed: the save flow and its baseSha guard; `saveBuffer`
and `loadBuffer`; the tree, palette, and tab strip; Markdown's block
rendering and every other inline token; the policy engine; the contracts in
`testing/feat-dock-shell.md` and `testing/feat-dock-changes.md`. The
design-system guard stays green with no new allowlist entries.
