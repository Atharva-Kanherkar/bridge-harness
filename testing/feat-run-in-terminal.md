# feat/run-in-terminal: test contract

A shell code block an agent writes in chat gets a **Run** button. One click
opens the dock's Terminal pane and runs the command in the chat's active shell.

## Functional Behavior

- Shell fences are `bash`, `sh`, `zsh`, `shell`, `console`, `shell-session`,
  `shellsession` and `terminal` (case-insensitive). Every other language
  (`ts`, `python`, `fish`, `powershell`, no language) gets no Run button.
- The Run button sits in the code block header, before Copy: a play icon and
  the label "Run". Its accessible name says where it runs ("Run in terminal").
- It renders only when the host provides a terminal (the main chat with a
  repository and workspace). Notes, previews, worker feeds and repo-less chats
  render no Run button; Copy is unchanged everywhere.
- Inside the chat it renders only in the agent's own reply prose: a command
  the agent suggests to the person. Markdown the chat shows for other reasons
  gets Copy alone: tool output, thinking, delegation directives, worker
  result summaries, PR descriptions, and outgoing drafts written for someone
  else (a shell line in a Slack draft is text to send, not a command to run).
- It renders only on a **closed** fence. A fence still streaming (no closing
  ```` ``` ```` yet) has Copy but no Run, so a half-written command can never run.
- What runs:
  - A *transcript* keeps only prompt lines (`$ `, `% `), prompts stripped, plus
    continuation lines after a line ending in `\` (a leading `> ` on those is
    stripped); output lines are dropped.
  - `bash`/`sh`/`zsh`/`shell` are a transcript when their first non-blank line
    starts with a prompt; otherwise the body runs as written.
  - `console`/`shell-session`/`shellsession`/`terminal` are a transcript when
    any line starts with a prompt; otherwise the whole body runs.
  - Leading and trailing blank lines are dropped. C0/C1 control characters
    other than tab and newline are stripped (what you see is what runs).
  - A body that reduces to nothing gets no Run button.
- Click flow: the dock opens on the Terminal pane; the command is written to
  the workspace's **active** shell in one `write_terminal` call, followed by
  Enter. Multi-line commands keep their line structure (`\n` → `\r`). When
  the shell enabled bracketed paste, the text is wrapped in
  `ESC[200~ … ESC[201~` so the shell takes it as one edit; Enter follows the
  closing marker.
- No shell yet, or the active shell exited: a new shell opens and the command
  runs there once its prompt has drawn (first output, then 150 ms of quiet,
  capped at 4 s).
- A run requested before the lazy Terminal pane has mounted is queued and runs
  when the pane mounts. A request no pane picks up within 10 s fails.
- Button states: idle "Run" → "Running…" while waiting → "Sent to shell 1"
  (shell label) with a check for ~2 s → idle. On failure: "Couldn't run" with
  the error in its title for ~3 s → idle. The button is disabled while running.
- The shell gets focus after the run.
- A shell's PTY is sized to its xterm once it has opened. Found in the real
  E2E: the first resize fired before the PTY existed and was dropped, so a
  fresh shell ran 120 columns wide in a narrow dock and zsh garbled the echo
  of a pasted command.

## Unit Tests

`src/terminal/runCommand.test.ts`
- `isShellFence` accepts the eight languages in any case, rejects others.
- `commandFromFence` strips `$ ` prompts in a bash transcript, keeps its
  continuation lines and drops its output lines; a bash body that does not
  start with a prompt runs as written.
- `commandFromFence` keeps only prompt lines and continuations for `console`.
- `commandFromFence` returns the whole console body when it has no prompts.
- `commandFromFence` strips control characters and trims blank edges; returns
  `null` for an empty result.
- `terminalRunInput` without bracketed paste: `\n` → `\r`, trailing `\r`.
- `terminalRunInput` with bracketed paste: wrapped, Enter after `ESC[201~`,
  and an embedded `ESC[201~` cannot survive (control chars stripped first).
- `terminalRunInput` without bracketed paste turns tabs into spaces (a tab
  would trigger completion); with bracketed paste tabs are kept.

`src/terminal/runQueue.test.ts`
- A request with no listener waits; a listener attached later drains it.
- A live listener receives a request immediately.
- Requests for another workspace are not delivered.
- An undrained request rejects after the timeout.
- `settle` resolves the requester's promise with the shell label.

`src/components/Markdown.test.tsx` (`splitBlocks`)
- A closed fence is `closed: true`; an unclosed trailing fence is `closed: false`.

## Integration / Functional Tests

`src/components/RunInTerminal.test.tsx` (jsdom)
- A `bash` block inside `RunInTerminalContext` renders a "Run in terminal"
  button; a `ts` block does not; without the context no button renders.
- An unclosed bash fence renders Copy but no Run.
- Clicking Run calls the context with the extracted command, shows
  "Running…", then "Sent to shell 1" when it resolves, then returns to "Run".
- A rejected run shows "Couldn't run" with the error message as the title.

`src/components/AgentConversation.test.tsx` (jsdom)
- With `onRunInTerminal`, a bash block in an assistant reply has Run; the same
  block in an outgoing draft card and in thinking has Copy only.

`src/components/TerminalPane.test.tsx` (jsdom, mocked `bridgeApi` + xterm)
- A run queued before mount is written to the active shell as one
  `writeTerminal` call ending in `\r`, after `openTerminal`, and resolves with
  the shell label.
- With the active shell marked exited, the run opens a new shell and writes there.
- Once `openTerminal` resolves, the shell sends `resizeTerminal` with its
  xterm's rows and columns.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (vitest + cargo + sidecar). No Rust change is expected.

## E2E Tests

- Mock mode (`bun run dev`): open a chat, click Run on a bash block → the dock
  opens on Terminal and the command echoes in the preview shell.
- Real app (manual): ask an agent for `echo hello && pwd` in a bash block,
  click Run → dock Terminal shows the command executing in the worktree.

## Manual / cURL Tests

N/A for cURL: no wire method changes. Manual steps above; screenshots of the
button in dark and light, and of the sent state with the terminal open.
