# Agent Fleet terminal workspaces

Agent Fleet now opens terminals running shells and installed agent CLIs in a selected workspace. Each workspace saves its own tabs, nested splits, pane titles, ratios, focus, and maximized pane. It uses Bridge's existing Graphite & Paper tokens and Tailwind v4.

Use **New Terminal** for a login shell or **New Agent** for Codex, Claude Code, OpenCode, Cursor, or Grok. Agent actions resolve an interactive executable; having a conversation SDK installed alone does not make its CLI available. A missing executable produces an installation error. Bridge preserves the CLI's normal trust, authentication, and permission prompts.

Pane headers support rename, split right/down, maximize, and close. Drag a pane header to another pane's edge to move it into a split; drop it onto a tab to move it between tabs. The toolbar can move the focused pane into a new tab. Dividers support pointer dragging and arrow keys, with Home restoring a 50/50 ratio. Small windows scroll the layout instead of shrinking panes below their minimum usable size.

| macOS shortcut | Action |
| --- | --- |
| Command+T | New terminal tab |
| Command+D / Command+Shift+D | Split right / down |
| Command+Shift+W | Close focused pane |
| Command+Shift+Return | Maximize / restore |
| Command+Option+Left / Right | Focus previous / next pane |
| Command+Shift+[ / ] | Previous / next tab |
| Command+F | Search scrollback |

On other platforms the application modifier is Control+Alt. Plain Control combinations, including Control+C, Control+D, and Control+F, belong to the terminal. Use Command+C to copy a selection and ordinary terminal paste. Web links open with Command-click (Control-click elsewhere).

## Reuse from Orca

The reference revision is [`f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7`](https://github.com/stablyai/orca/tree/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7). Bridge adapts Orca's recursive split layout, WebGL disposal, absolute-cursor serializer, partial-escape tracking, and mouse-mode mirror. The host history runtime follows Orca's headless xterm, sequenced output, resize journal, and checkpoint design. [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) preserves the MIT license and attribution; the packaged helper includes it too.

Rendering uses Orca's pinned xterm/addon versions, with the same Unicode 11 tables in the renderer and headless emulator. Orca's Electron-specific GPU configuration is not applied to Tauri's WKWebView.

## Process and history lifecycle

The Rust host owns PTYs independently of mounted React panes. Switching tabs, moving panes, or leaving Agent Fleet does not close a process. Reattaching to a live terminal retains its ID and generation. Only creating a terminal or explicitly restarting an ended terminal launches a process. Explicit Restart begins a new generation with fresh history; merely reopening the app does not restart an agent.

Bridge's existing launcher stops a **desktop-owned daemon** when the app exits. The embedded host also stops with the app. Their layouts and scrollback return as ended panes with a Restart action. When Bridge attaches to a **separately running daemon** that survives desktop exit, its live terminals can be reattached. Daemon failure or machine restart restores history, not process memory. This change does not alter the launcher's existing shutdown or build-identity protection.

The Node helper stores terminal state beside `bridge.db`, under `terminal-state/`, outside repository files. It atomically installs VT checkpoints and replays a sequenced journal. Raw UTF-8 bytes are decoded across PTY chunks, including a pending character at checkpoint time. Snapshots preserve scrollback, alternate screen, cursor state, partial escapes, dimensions, mouse encoding, and negotiated Kitty keyboard flags. Renderer attachment subscribes before requesting a snapshot; only newer frames are applied. Lag, reconnect, and generation changes trigger recovery.

Budgets are 5,000 scrollback lines, a 512 KiB journal checkpoint threshold, and a 4 MiB ANSI snapshot limit per terminal. Old ended histories are trimmed toward a 128 MiB aggregate storage target; running sessions are exempt from aggregate pruning, so their combined storage can exceed that target. The renderer uses at most eight WebGL contexts, falls back to the DOM renderer, and retains no GPU contexts for inactive tabs. Ended history reads release their headless emulators. Node.js is required; the helper and exact npm dependencies are staged into the app bundle by `prepare:terminal-state`.

The terminal RPCs and transient sequenced frame notification are part of protocol **1.8**. Generated schemas and TypeScript declarations are checked in; a 1.7 daemon is rejected before the desktop uses the new contract.

## Verification

Verified on 2026-09-10:

- Production frontend build, native desktop/daemon build, and packaged helper import/write/snapshot smoke test.
- Repository checks: 2,065 frontend tests, 2,278 core tests, 319 remaining Rust tests, 78 Node tests, and 12 Python release tests passed. Fourteen existing tests were skipped/ignored. The full run exposed notification-fixture and desktop-contract gaps; those were fixed, with the affected suites rerun. Commands used the repository's npm scripts because the local Bun launcher failed with `CouldntReadCurrentDirectory` before running a script.
- Twelve helper tests covering UTF-8 boundaries, unattended journal recovery, alternate screen/cursor continuity, partial escapes, mouse/keyboard modes, resize order, stale generations, checkpoint deduplication, close/exit races, layout bounds, and ended-emulator disposal.
- Real PTY core tests covering unattended output, live reattachment without respawn, helper restart, stale running records after host restart, ended restoration, explicit restart, invalid launch inputs, and close.
- An isolated rebuilt daemon with two real shells: independent output, output produced without a client, unchanged generations on reconnect, final 77-column/23-row size after repeated resizes, saved split layout and history after daemon restart, no automatic process restart, explicit restart creating a new generation, and close removing the panes.
- An installed Codex CLI launched in the isolated checkout and displayed its interactive directory-trust prompt. No trust approval or model prompt was submitted.
- Production Chrome preview: nested splits, scrollback search, closing search, maximize/restore, rename, moving a pane into another tab, ten consecutive tab switches, and minimum pane dimensions at 800×600. Screenshots were visually inspected. A forced WebGL context loss removed that pane's GPU canvases and retained its visible output with the DOM renderer; both panes remained present and no browser runtime errors were recorded.

Browser preview uses an explicitly labeled mock terminal transport. It verifies controls and rendering; real process checks above use the native daemon. Chrome's accessibility-snapshot command intermittently stalled after pane remounts; final browser verification used DOM actions/assertions and screenshots, which completed without the stall. Focus is explicitly released before a terminal input leaves the DOM, with regression coverage for that ordering.

macOS lock prevented completion of WKWebView keyboard/mouse interaction testing. Native accessibility behavior, WebGL performance, modifier behavior in WKWebView, and full interactive sessions in every supported CLI remain unverified; successful Chrome checks are not evidence for those claims.
