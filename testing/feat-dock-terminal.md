# feat/dock-terminal — test contract

Locked before implementation. One workstream: the terminal pane grows from
one shell per workspace into several, each a real PTY in the workspace's
worktree, with the wire contract to match. A dev server, a test watcher,
and an ad-hoc prompt are three shells, not one. Closing a terminal is an
explicit act; no UI toggle ever kills a process, and terminal bytes stay
transient notifications — never durable events, never agent context.

## Shape of the thing

The protocol's terminal domain learns identity: open, write, and resize
carry a terminalId beside the workspaceId, close_terminal ends one shell
deliberately, and list_terminals names the shells still running for a
workspace, so a reopened window reattaches instead of respawning. The
session-output notification carries the terminalId, and a new transient
terminal-exited notification says which shell ended. In bridge-core the
runtime key becomes terminal:workspace:terminal, the reader thread reports
the exit before it cleans up, and close kills the child. Artifacts are
regenerated, never hand-edited.

In the pane, a strip inside the dock lists the workspace's shells: create,
switch, rename (a frontend label), and close with each shell's xterm kept
mounted so switching never repaints history. Scrollback keys by workspace
and terminal with the same LRU budget. The footer states the worktree path
and the scrollback bound. Output arriving on a shell that is not on screen
marks its tab; a shell that exits says so; the pane reports activity upward
and App turns it into the running count and attention mark on the dock
switcher. A direct chat still explains that a terminal needs a workspace.

## 1. The wire contract — bridge-protocol inline tests

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Terminal params round-trip with identity | open/write/resize serialize workspaceId + terminalId camelCase and round-trip |
| 1.2 | Close and list round-trip | close_terminal params and list_terminals params/result round-trip; terminalIds is a string array |
| 1.3 | Incomplete payloads are refused | a missing terminalId fails deserialization for open, write, resize, and close |
| 1.4 | The registry names the new surface | methods include terminal/close_terminal and terminal/list_terminals; notifications include terminal-exited as Transient |

## 2. The runtime — bridge-core

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Session output carries the shell | CoreEvent::SessionOutput payload serializes sessionId, terminalId, and data |
| 2.2 | Exit is an event, not a silence | CoreEvent::TerminalExited payload serializes sessionId and terminalId, mapping to terminal-exited |
| 2.3 | Shells key by workspace and terminal | two opens with distinct terminalIds coexist; write and resize address exactly one |

## 3. The pane — `src/components/TerminalPane.test.tsx` (xterm mocked)

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | The strip lists shells and creates one on demand | first mount opens t1; the new-shell control opens t2 and switches to it |
| 3.2 | Reattach instead of respawn | with list_terminals reporting live shells, mount shows them and opens nothing |
| 3.3 | Output routes by shell | a chunk for t2 writes to t2's terminal only |
| 3.4 | Switching never repaints history | shells stay mounted across switches (same host nodes) |
| 3.5 | Close is explicit | the tab's close control calls close_terminal; nothing else does |
| 3.6 | Background output marks the tab | a chunk for an inactive shell shows its activity dot; activating clears it |
| 3.7 | An exit is visible | terminal-exited flips the tab's mark to exited state |
| 3.8 | Activity reports upward | the callback receives the running count and whether attention is due |
| 3.9 | The footer states place and bound | the worktree path and the scrollback limit render |
| 3.10 | Renaming is a label, not a protocol call | renaming a tab changes its label and calls no api |

## 4. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The third chord opens the multi-shell pane | the shell strip renders inside the dock for a repo session |
| 4.2 | The switcher carries terminal activity | with attention reported, the terminal tab shows the alert dot while another pane is active |

Explicitly **not** changed: terminal bytes remain Transient notifications
excluded from the session forest and the agent's context; the PTY spawn
environment (zsh -l, TERM, BRIDGE_WORKSPACE_ID); scrollback's LRU budget
values; the dock shell lifecycle; the policy engine; prior dock contracts.
Generated protocol artifacts change only through the generator. The
design-system guard stays green with no new allowlist entries.
