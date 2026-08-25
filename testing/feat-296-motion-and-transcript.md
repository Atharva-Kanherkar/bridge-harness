# feat/296-motion-and-transcript — test contract

Locked before implementation. Three workstreams: Framer Motion for overlay exits,
motion polish in the transcript, and the three-layer tool card with inline diffs.

## Scope note on animation under test

Framer Motion drives real `requestAnimationFrame` work, which jsdom does not run
in a way we want to assert on. The contract therefore asserts **lifecycle and
structure** — is the node still in the DOM, does the fold bar exist, does the
chip carry the right text — never a computed frame. Where a test needs motion out
of the way it sets `MotionGlobalConfig.skipAnimations = true`, which makes every
animation resolve on the first frame.

`useReducedMotion()` reads `prefers-reduced-motion`. Under it, transitions carry
`duration: 0` rather than a different set of variants: the same code path runs,
it just lands immediately. That is what keeps "reduced motion" from becoming a
second, untested rendering.

## 1. Overlay exit animations

`CreateDialogShell` owns the open/closed boundary. Each dialog keeps its own
`open` prop and its own focus/Escape behaviour, but no longer returns `null`
before the shell renders.

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | A shell rendered with `open={false}` has nothing in the DOM | no `[role="dialog"]` |
| 1.2 | A shell rendered with `open` shows the scrim, panel, title and children | `[role="dialog"]` present, `aria-modal="true"`, `aria-labelledby` matches the heading id |
| 1.3 | Flipping `open` to `false` keeps the dialog mounted while it exits | `[role="dialog"]` still present immediately after the re-render |
| 1.4 | The exit completes and the tree unmounts | with `skipAnimations`, `[role="dialog"]` is gone after the exit settles |
| 1.5 | The close button still calls `onClose` and honours `closeDisabled` | click → callback; disabled attribute set |
| 1.6 | `dismissOnScrim` still closes on a scrim click and on Escape, and ignores clicks inside the panel | callback counts |
| 1.7 | The panel no longer carries the CSS entrance class | no `animate-page-enter` in the panel's class list (Framer owns the entrance now) |
| 1.8 | Every dialog built on the shell passes `open` through instead of gating itself | `NewChatDialog`/`WorkspaceCreateDialog`/`OrchestratorCreateDialog` with `open={false}` render nothing |

Sidebar drawer:

| # | Behaviour | Assertion |
|---|---|---|
| 1.9 | The mobile scrim is present while `mobileOpen` | `button[aria-label="Close navigation"]` exists |
| 1.10 | Closing keeps the scrim mounted for its fade, then removes it | present right after the re-render; absent once the exit settles |
| 1.11 | The drawer keeps its existing Tailwind `transition-transform` slide | `-translate-x-full` / `translate-x-0` still on the `<aside>` |

Explicitly **not** changed: `transition-colors`, `active:scale-*`, and every other
Tailwind micro-interaction. `bun run test` must show no new snapshot of them.

## 2. Conversation motion

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Transcript items are Framer-driven wrappers, not CSS `chat-message-enter` | a rendered transcript has no `.chat-message-enter` on message rows |
| 2.2 | An arriving item does not restart the entrance of the items already on screen | `AnimatePresence initial={false}` — existing rows keep their identity across a re-render with one appended item (assert by node identity) |
| 2.3 | Expanding a tool row animates its body in and keeps it mounted while collapsing | body present after the collapse click, gone once the exit settles |
| 2.4 | An `ActivityGroup` collapse animates the same way | same lifecycle on the group body |
| 2.5 | A running tool shows the spinner; a completed one shows the success tick, swapped through `AnimatePresence` | exactly one status glyph at a time |
| 2.6 | An `error` item renders with the firmer error entrance and its described title | title text present, `role="alert"` retained |
| 2.7 | A pending approval's action row is replaced by the resolved line when the status changes | buttons gone, status line present |
| 2.8 | Nothing added here introduces a palette class, hex literal, or blur on a resting surface | `src/designSystem.test.ts` stays green |

## 3. Three-layer tool card and inline diffs

### `toolCallDisplay()` in `src/conversation.ts`

A typed reader over one `ConversationItem`. Pure, no React.

| # | Input | Expectation |
|---|---|---|
| 3.1 | Claude `Bash` tool_use | `verb: "run"`, `command` set, `glyph: "terminal"` |
| 3.2 | Claude `Edit` with `input.file_path` | `verb: "edit"`, `target` is the basename, `path` the full path |
| 3.3 | Codex `commandExecution` with `exitCode: 0` | `exitCode === 0` |
| 3.4 | Codex `commandExecution` with `exit_code: 2` | `exitCode === 2` (snake case tolerated) |
| 3.5 | An item with no exit code anywhere | `exitCode === undefined` — the chip must degrade, not render `exit NaN` |
| 3.6 | `file_change` with `data.patch` | `patch` returns it verbatim |
| 3.7 | `file_change` whose diff sits in `data.changes[].diff` | patches joined in order |
| 3.8 | A diff-bearing item whose only body is `item.text` | `patch` falls back to the text when it looks like a diff |
| 3.9 | `additions`/`deletions`/`durationMs` | surfaced as numbers, `undefined` when absent or unparseable |
| 3.10 | Status | `"running"` for `inProgress`/`streaming`, `"failed"`, `"completed"`, else `"idle"` |

### `PatchView` hunk folding in `src/components/DiffView.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 3.11 | A two-hunk patch with `foldAfterHunks={1}` renders the first hunk and a fold bar | one `@@` header visible; bar text matches `/1 more hunk .*expand/` |
| 3.12 | Clicking the fold bar reveals the rest | both `@@` headers visible, bar gone |
| 3.13 | A single-hunk patch renders no fold bar | no `button` inside the patch |
| 3.14 | A fragment with no `@@` header renders as one group, unfolded | rows present, no fold bar |
| 3.15 | The patch is no longer truncated at 8,000 characters | a long patch's last line is reachable after expanding |

### Rendering in `AgentConversation`

| # | Behaviour | Assertion |
|---|---|---|
| 3.16 | A `file_change` item with a patch renders its diff inline without a click | added/removed rows in the DOM on first render |
| 3.17 | A command item renders an `exit 0` chip in the summary row | text `exit 0` present; `text-success` on it |
| 3.18 | A failing command renders `exit 2` in the destructive tone | text `exit 2`, `text-destructive` |
| 3.19 | A command with no exit code renders no chip | no `/^exit /` text |
| 3.20 | An expanded command shows the `❯` prompt line and the output on the code ground | prompt glyph present, command text present |
| 3.21 | Reads and searches render as flat rows under an uppercase group label | label text "Explored" present; those rows carry no card border |
| 3.22 | Edits and commands render as bordered cards | the row's wrapper carries `border` |
| 3.23 | `mockConversation.ts` exercises the new rendering | `bun run dev` preview shows an inline diff and an exit chip (manual) |

## Gates

`bun run check`, `bun run test`, `bun run build` all green, `src/designSystem.test.ts`
included.
