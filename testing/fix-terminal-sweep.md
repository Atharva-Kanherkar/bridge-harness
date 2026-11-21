# fix/terminal-sweep: test contract

Locked before implementation. One workstream: every terminal surface in
Bridge (the dock's shell pane, the Agent Fleet split workspace, and the
provider sign-in pane) renders full-screen TUIs correctly and stops losing,
reordering, or garbling bytes. The dock pane stops running its own xterm
engine and uses the same checkpoint + sequenced-frame surface as Agent
Fleet, so both get the same fixes once.

## Bugs this branch fixes

| # | Surface | Bug | Root cause |
|---|---|---|---|
| T1 | Dock | A bare LF from a program that turned output translation off returns to column 0 | xterm `convertEol` adds a CR the PTY never sent. Minor: vim, top, less, nano, and curses replayed identically with it on or off |
| T2 | Dock | Emoji, CJK, and wide glyphs misalign | no Unicode 11 width table, unlike Agent Fleet and the PTY's own wcwidth |
| T3 | Dock | Closing the dock or switching panes wipes the screen | a hidden host measures 0x0, FitAddon proposes 2x1, xterm resizes to it; the PTY rejects 1 row so the program never redraws. Chrome: a `top` screen is reduced to one line after a hide and show |
| T4 | Dock | Reopening the pane replays garbage | it replays a raw byte log cut at 1 MB (mid escape sequence) into a terminal of another size, instead of the runtime's screen checkpoint |
| T5 | Dock | The pane ignores PTY resizes made elsewhere | no sequenced frames; Agent Fleet resizing the same shell leaves the dock at the wrong width |
| T6 | Dock | Exited shells reject input silently and cannot restart | unhandled write rejection; no restart control |
| T7 | Dock | Live terminals from Agent Fleet show as raw UUID tabs | labels derived from the id, not the record title |
| T8 | Both | The bottom row and right column of every TUI are clipped | Tailwind's border-box makes FitAddon read the padded height as available space |
| T9 | Both | Fast typing and key repeat arrive out of order or get dropped | each keystroke is its own concurrent IPC; the daemon proxy runs them on a thread pool and rejects floods as "busy" |
| T10 | Both | Mouse clicks and the wheel do nothing in ncurses apps (htop, mc, Python curses) | macOS ncurses enables only `?1000h` (terminfo `kmous=\E[M`), so xterm reports clicks in X10 encoding on `onBinary`, and nothing listened. vim negotiates SGR and was never affected |
| T11 | Both | No way to select text inside a mouse-mode TUI on macOS | `macOptionClickForcesSelection` is off, so Option-drag cannot override mouse tracking |
| T12 | Fleet | The bottom rows of a new pane are drawn out of sight | xterm measures cells before Geist Mono has loaded, fits too many rows, and never refits when the font arrives. Chrome: 40 of 41 cold loads |
| T13 | Both | A terminal opened while hidden never recovers | every background dock tab opens hidden. Chrome: a `top` screen shows one line once the tab is shown |
| T14 | Both | ANSI black is invisible in light mode; ANSI yellow renders gray | palette maps black to `--muted` (the light page colour) and yellow to the achromatic `--warning` |
| T15 | Fleet | Splitting, closing, or moving a pane reloads every other terminal in that tab | the split tree changes shape, React remounts each pane, xterm is disposed and the snapshot restored |
| T16 | Runtime | One program that stops reading input freezes every terminal and session stop | `write_terminal` writes to the PTY while holding the global runtimes mutex |
| T17 | Runtime | Resizing holds the global runtimes mutex across two Node round trips | lock order in `resized` |
| T18 | Runtime | Closing a terminal whose program ignores SIGHUP leaves a zombie | `kill` escalates to SIGKILL and nothing waits. The reader now also reaps and records the exit before it unregisters the runtime |
| T19 | Runtime | Fleet agent CLIs exit at once in a Finder-launched app | agent PTYs get the GUI PATH, so `#!/usr/bin/env node` fails |
| T20 | Runtime | TUIs fall back to ASCII or mis-measure Unicode | no UTF-8 locale in the PTY environment when Bridge is launched from Finder |
| T21 | Sign-in | Replacement characters at chunk boundaries | provider login output is decoded per 8 KB chunk with `from_utf8_lossy` |
| T22 | Runtime | A signal during a PTY read ends a live terminal | `EINTR` from `read` is treated as end of stream |
| T23 | Both | A black band under the last row | xterm 6 leaves `.xterm-viewport` at xterm.css's `#000`; it no longer takes the theme background |

## 1. Runtime: bridge-core inline tests

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Writes never hold the shared runtime map | with a terminal whose writer blocks, `list_terminals` and a write to a second terminal return promptly |
| 1.2 | Writes stay ordered | 200 sequential single-character writes arrive in order in the PTY output |
| 1.3 | Binary writes are bytes, not UTF-8 | a binary write of U+00FF reaches the PTY as the single byte 0xFF |
| 1.4 | A normal exit keeps its exit code | `exit 3` records status `exited` with `exitCode` 3, never the snapshot path's code-less recovery |
| 1.5 | Close reaps its child | after `close_terminal` on a child that ignores SIGHUP, signal 0 to its pid fails (no zombie) |
| 1.6 | PTY environment | a spawned shell sees a UTF-8 `LANG` when none is set, keeps an existing locale, and gets the hydrated PATH |
| 1.7 | Incremental UTF-8 | a decoder fed `e2 94` then `80` yields one `─`, never U+FFFD |
| 1.8 | Existing contract | the three existing `terminal_workspace` tests still pass |

## 2. Wire: bridge-protocol

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | `binary` is optional | WriteTerminalParams without `binary` deserializes as `false`; with `binary: true` round-trips |
| 2.2 | Artifacts | regenerated by the generator; `checked_in_artifacts_match_the_contract` passes |

## 3. Input queue: `src/terminal/input.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | One write in flight per terminal | a second send while the first is pending does not call the api |
| 3.2 | Coalesced and ordered | "a","b","c" sent during one in-flight write arrive as "a" then "bc" |
| 3.3 | Binary is never merged with text | text, binary, text produce three writes in order, the middle one with `binary: true` |
| 3.4 | A failure is reported, the queue continues | a rejected write calls the error handler and the next send still goes out |

## 4. Engine: `src/terminal/engine.test.tsx` (xterm mocked)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | No fit while hidden | a 0x0 host never resizes xterm or the PTY |
| 4.2 | The xterm parent carries no padding | the element passed to `open` has no padding classes |
| 4.3 | Binary input is forwarded | an `onBinary` event writes with `binary: true` |
| 4.4 | Options a TUI needs | Unicode 11 active, no `convertEol`, `macOptionClickForcesSelection` on |
| 4.5 | Remount reuses the engine | release then acquire of the same terminal generation within one task returns the same xterm without a second snapshot |
| 4.6 | Release disposes | release without a re-acquire disposes xterm and unsubscribes |
| 4.7 | Opens only once fonts and size are ready | `open` waits for `document.fonts.load` and a non-zero host |
| 4.9 | A view reclaims the PTY size | after a resize frame from another view, this view's next fit sends its own size again |
| 4.8 | A split keeps the live xterm | re-rendering a surface under a new parent keeps the same xterm and takes no second snapshot; unmount disposes it |

## 5. Palette: `src/terminal/theme.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | Light mode black is dark | with light tokens, `black` is not the page background |
| 5.2 | Yellow is a colour | `yellow` differs from `white` in both modes |

## 6. Dock pane: `src/components/TerminalPane.test.tsx` (surface mocked)

| # | Behaviour | Assertion |
|---|---|---|
| 6.1 | Contract §3 of `feat-dock-terminal.md` holds | create, reattach, switch, close, marks, rename, activity |
| 6.2 | The pane renders the shared surface | each shell mounts `TerminalSurface` with the record `createTerminal` returned |
| 6.3 | Output never goes to xterm twice | session-output chunks only mark tabs; no direct xterm writes |
| 6.4 | Exited shell restarts on request | the Restart control calls `createTerminal` with `restart: true` |
| 6.5 | Fleet terminals read as their title | a live non-`tN` id is labelled with its record title |
| 6.6 | Footer states the real bound | "5,000 lines scrollback" |
| 6.7 | Run in terminal still works | queued runs, bracketed paste, and prompt wait keep their tests |

## 7. Fleet: `src/components/TerminalWorkspace.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 7.1 | Existing contract | all existing cases pass |

## 8. Real-engine checks (throwaway harness, headless Chrome at 2x, not committed)

| # | Check | Expected |
|---|---|---|
| 8.1 | Bottom row fits across 41 pane heights (400 to 640 px) | after: 41 of 41 correct |
| 8.2 | Hide then show a pane running vim or top | after: screen intact |
| 8.3 | Open a pane while hidden, then show it | after: full screen |

| 8.4 | Real `bridged` (started without `LANG`), real PTY, the new surface | `stty size` equals the xterm grid before and after a resize; `LANG` is UTF-8; vim fills the screen; `exit 7` records exit code 7 |
| 8.5 | Click inside a real ncurses program (`?1000h`, X10 encoding) | the new engine delivers it; main's surface does not |

## 9. Gates

`bun run build`, `bun run test`, `node --test sidecar/terminal-state/state.test.mjs`.

Explicitly **not** changed: terminal bytes stay transient notifications,
never forest events or agent context; the checkpoint sidecar's storage
format; the layout tree format; the shortcut table; the provider sign-in
UI. Generated protocol artifacts change only through the generator.
