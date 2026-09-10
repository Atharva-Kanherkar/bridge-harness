# Mission Control terminal splits

Research date: 2026-09-10. Status: approved design, implemented. See [usage, lifecycle, and verification](../mission-control-terminal-splits.md) for the delivered behavior and validation limits. The sections below retain the original research and design.

The user confirmed that Mission Control should contain **terminals running agent CLIs**. The target is a persistent terminal workspace with nested splits, direct keyboard input, and independent agent processes in each pane.

## Reference findings

Orca's terminal is built on xterm.js. Its split layout uses stable leaf IDs, recursive split nodes, and saved ratios. Tabs can contain terminal splits, and worktrees retain their own layouts. These are the interaction patterns to adopt. Sources: [Terminal](https://www.onorca.dev/docs/terminal), [Tabs, panes and split layouts](https://www.onorca.dev/docs/model/tabs-panes-splits), and [terminal layout types](https://github.com/stablyai/orca/blob/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7/src/shared/terminal-tab-types.ts).

Orca's WebGL integration includes context-loss recovery, a DOM fallback, and a bounded cache of hidden GPU contexts. GPU acceleration alone is not the whole terminal implementation. Sources: [WebGL renderer](https://github.com/stablyai/orca/blob/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7/src/renderer/src/lib/pane-manager/pane-webgl-renderer.ts) and [hidden-context retention](https://github.com/stablyai/orca/blob/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7/src/renderer/src/lib/pane-manager/terminal-webgl-hidden-retention.ts).

Orca's current daemon maintains a headless terminal emulator and sequenced output records, including resize operations. History uses checkpoints plus an output log; reaching the log budget requests a new checkpoint. This allows recovery to reconstruct terminal state rather than displaying a sliced text tail. Sources: [session output handling](https://github.com/stablyai/orca/blob/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7/src/main/daemon/session-output-plane.ts) and [history writer](https://github.com/stablyai/orca/blob/f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7/src/main/daemon/terminal-history-session-writer.ts).

App restart and machine restart have different contracts. A surviving Orca daemon permits reattachment to live processes. A reboot or daemon failure ends those processes; saved layout and history can still return. Source: [Session restore](https://www.onorca.dev/docs/model/session-restore).

Source inspection was pinned to Orca commit `f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7`. Orca itself was not installed or run.

## Bridge today

| Area | Existing foundation | Required change |
| --- | --- | --- |
| Mission Control | `src/components/MissionControl.tsx` projects agent sessions into status tiles. | Render an interactive terminal workspace with an explicit navigation entry. |
| Terminal rendering | `src/components/TerminalPane.tsx` uses xterm.js and FitAddon, with several shell tabs. | Extract a reusable terminal surface; add WebGL with recovery and a fallback. |
| Process ownership | `src-tauri/bridge-core/src/api.rs` owns PTYs by workspace and terminal ID; opening a running ID is idempotent. | Persist terminal records and separate creating a process from attaching a view. |
| History | `src/terminalScrollback.ts` is a bounded JavaScript map. Terminal notifications are transient and have no replay cursor. | Record output in the host, with terminal-state checkpoints and ordered replay. |
| Layout | `src/dockLayout.ts` stores one dock's dimensions and selection. Terminal tab names and roster live in memory. | Persist terminal tabs, a recursive split tree, ratios, titles, and focus per workspace. |
| CLI discovery | `src-tauri/bridge-core/src/managed_agents.rs` resolves managed and external runtimes. | Resolve and validate an interactive CLI launch target; SDK availability alone does not establish a usable terminal CLI. |

Bridge can attach to a separately running daemon. Its desktop shutdown disconnects the client and joins its connection supervisor. Native verification also confirmed that the launcher terminates a daemon it owns on app exit. Live reattachment therefore requires a surviving, separately managed daemon; otherwise the saved history returns with an ended state. The embedded fallback also ends with the desktop process.

## Proposed user experience

1. **Mission Control opens a terminal workspace.** A workspace selector chooses the checkout. Saving layouts per workspace/worktree is the proposed default, following Orca.
2. **New Terminal and New Agent are visible actions.** Agent presets launch an installed interactive CLI in the chosen checkout. A shell pane permits arbitrary commands. A missing CLI produces an actionable setup state.
3. **Split Right and Split Down operate on the focused pane.** Splits nest, separators resize, and panes can be moved, renamed, maximized, and closed. A split creates a distinct terminal identity and process. It inherits the source pane's confirmed current directory when available, otherwise the workspace directory.
4. **The pane header identifies its work.** Show the agent or shell, title, checkout/branch, and observed process state. Keep keyboard focus apparent. Use reliable CLI integration events for richer states such as waiting for input; a lack of output does not establish that state.
5. **Navigation preserves running work.** Switching tabs or leaving Mission Control detaches or hides presentation without stopping the terminal. Explicit close ends the addressed terminal according to the product's close semantics.
6. **Reopening restores place and history.** Restore the split tree, ratios, selected tab and pane, titles, and scrollback. Reattach when the host still owns the same process. When it has ended, retain history and expose Restart; navigation must not silently relaunch an agent.
7. **Terminal ergonomics are part of the feature.** Include scrollback search, copy/paste, selection, working links, and shortcuts for tabs, splits, pane focus, and maximize. Preserve terminal control keys and test modifier-aware input with the supported CLIs.

The layout has no fixed two- or four-pane grid. Minimum usable pane sizes, bounded history, and rendering budgets still apply. Existing Graphite & Paper tokens and Tailwind v4 remain the styling system.

## Implementation boundaries

### Layout and presentation

Add a pure terminal-layout model with stable node IDs. A leaf refers to a terminal record; a split has an axis, a ratio, and two children. Persist workspace layouts with a schema version and explicit active tab, focused pane, and maximized pane IDs. Closing a leaf collapses its redundant split parent and repairs focus.

Extract xterm mounting, fitting, theme application, and renderer recovery from `TerminalPane.tsx` into a shared terminal surface. Keep terminal connections and byte delivery outside the pane component's mount lifetime. Moving a leaf must retain its terminal identity and must not spawn a replacement process. Only the focused pane receives keyboard input; define a single resize owner if a terminal is displayed in more than one surface.

Expose Mission Control through its own route and correctly labelled sidebar control. The current footer button labelled Usage invokes Mission Control. Update navigation and keyboard tests that currently require Mission Control to be hidden.

### Host terminal records and launch

Introduce durable terminal metadata: terminal ID, workspace ID, process generation, launch intent, starting/observed directory, title, lifecycle, and exit information. Keep terminal processes distinct from Bridge's SDK-driven conversation sessions; existing worker-result and approval state cannot be inferred from a CLI's text stream.

Use the existing runtime resolver for CLI discovery, then validate that the resolved target supports interactive launch. Use structured executable/argument/environment values. Preserve ordinary CLI authentication and permissions. Initial launch or explicit Restart creates a new generation; attaching or moving a pane never does.

### Output recovery

The host must collect terminal output even with no desktop connected. Add a bounded output journal and checkpoint representation that retains terminal screen state, scrollback, dimensions, and an output cursor. Include resize/clear operations in replay order. Preserve UTF-8 across PTY read boundaries; current per-read lossy conversion can split a multibyte character.

Attachment returns a checkpoint and sequence boundary, followed by newer output exactly once. Use process generations to reject delayed bytes or exits from a previous incarnation. A lag marker must lead to terminal recovery as well as conversation recovery.

Before implementing persistence, validate a headless VT-state engine for the Rust host against xterm rendering and the supported CLIs. This is the remaining library-selection decision: alternate-screen behavior, cursor modes, wrapping, Unicode, and resize fidelity must pass a compatibility fixture before selecting it. A raw text-tail cache cannot satisfy that contract.

Persist checkpoints atomically and bound both per-terminal and total storage. Keep history outside repository files. After daemon failure or reboot, restore saved content with an explicit ended state. Do not claim that process memory or an interactive agent survives a machine restart.

Add the necessary typed terminal RPCs and notifications to `bridge-protocol`, regenerate protocol artifacts, and bump compatibility according to its additive/breaking rules. A new desktop must reject an incompatible old daemon before presenting controls that depend on the new contract.

### Rendering and input

Use xterm's WebGL addon when the native webview supports it and fall back cleanly on failure. Bound retained hidden GPU contexts and batch terminal output outside React state updates. Coalesce resize work while preserving the final PTY dimensions. Native macOS WKWebView behavior must be measured; Orca's Electron-specific GPU settings cannot be assumed to transfer.

Keyboard handling belongs to the focused terminal except for explicit Mission Control shortcuts. Rich agent-state hooks can follow basic terminal lifecycle support; success must not be inferred from an exited shell or arbitrary agent prose.

## Delivery order and evidence

1. **Prove terminal recovery.** Select the host VT engine, add terminal metadata/generations and checkpoint/replay, and test output while no UI is attached.
2. **Build the terminal workspace.** Add the split-tree reducer, persistence, shared terminal surface, resize/focus/move/close behavior, and the Mission Control route.
3. **Add the agent launch experience.** Resolve interactive CLIs, show launch and exit failures, and implement explicit restart and pane identity.
4. **Finish rendering and ergonomics.** WebGL recovery, search, shortcut integration, background rendering budgets, and narrow-window behavior.
5. **Verify in the desktop.** Exercise nested splits with a real interactive CLI, navigate away and return, quit/reopen with a surviving daemon, and recover after a controlled test-daemon restart. Use temporary workspaces for destructive lifecycle fixtures.

Acceptance must cover independent input/output per pane; unchanged process identity across tab moves; final dimensions after rapid resize; retained output while the UI is absent; ordered replay without duplicate bytes; UTF-8 and alternate-screen fixtures; corrupt-layout recovery; bounded memory/disk/GPU use; and stale-daemon rejection. Run the repository-required build and test suites before a PR.

Research baseline: the existing `TerminalPane`, `terminalScrollback`, and `dockLayout` suites passed: **34 tests across 3 files**. These establish the existing foundation only; they do not validate the proposed split or restart behavior.
