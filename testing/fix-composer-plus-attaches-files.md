# fix/composer-plus-attaches-files — Test Contract

Locked before implementation. Two defects, both discovered by using the app.

## 1. The `+` control does the wrong thing

The previous change made the dock `+` do what its label said — open the
workspace dialog. That was the wrong reading of intent. In every comparable tool
(Claude Code, Codex) `+` beside a composer means **add context**: attach a file
to the message you are writing. Creating a workspace or a worktree from it is a
structural action nobody wants from a text box.

The issue that prompted the earlier change said as much: *"If the intended
product action has changed to an add-context menu, change the icon label and
implement that menu."* This does that.

### Expected behavior

- The dock `+` opens the workspace file picker — the same list `@` already
  drives — and does not clear the draft.
- Picking a file inserts it as an `@path` mention, so the existing backend path
  (`workspace_files::mention_context`) resolves it into trusted application
  context at submit time. No second attachment mechanism.
- The caret returns to the composer so the picker can be filtered by typing.
- The control's accessible label matches what it does on that surface: "Attach a
  file" on the dock.
- Where there is nothing to attach, the control is disabled **with the reason as
  its tooltip** — never a live control that does nothing:
  - no folder connected → "Connect a folder to this chat to attach files from it"
  - folder connected, file list not yet read → "No files to attach yet…"
- The welcome/hero composer has no conversation and no folder, so nothing to
  attach to. It keeps the structural action and keeps saying "New workspace".
- `+` stays reachable while the agent is working.

## 2. `tauri dev` restarts itself, which reads as the app hanging

`beforeDevCommand` runs `prepare:browser-host:dev` and `prepare:daemon:dev`,
which stage freshly built sidecars into `src-tauri/binaries/`. That directory is
inside the crate the dev watcher watches, so staging them looks like a source
edit: the watcher rebuilds and restarts the app, which runs `beforeDevCommand`
again. Observed in a real launch — one full rebuild-and-restart cycle
immediately after the app came up, before it was usable.

### Expected behavior

- `src-tauri/.taurignore` excludes `binaries/` and `target/`, so the dev watcher
  no longer treats the dev command's own output as a source change.
- A `tauri dev` launch reaches a running app and stays there, with no
  "File … changed. Rebuilding application…" line for a path the dev command
  itself wrote.

## Unit Tests

### Frontend

- `ComposerPill` — the `+` calls its handler and leaves a non-empty draft
  untouched (existing, still true).
- `ComposerPill` — the accessible label and tooltip come from `plusLabel`, so a
  surface cannot silently disagree with its own control. Default is "Attach a
  file".
- `ComposerPill` — `plusUnavailableReason` disables the control and becomes its
  tooltip.
- `ComposerPill` — `+` stays reachable while `working` (existing).
- `App` — the welcome `+` still opens the workspace dialog and preserves the
  draft (existing test, retitled to name the surface).

## Integration / Functional Tests

Attaching from the dock composer is the `@`-mention path, which already has
coverage in `fileMentions.test.ts` (query parsing, quoting, insertion) and in the
Rust `workspace_files` mention-context tests. This change routes a button into
that path rather than adding a mechanism, so no new integration surface exists to
contract.

## Smoke Tests

- `bun run build` — green.
- `bun run test` — green.

## Manual / Live Verification

1. Open a chat with a connected folder, type a partial message, click `+`: the
   file list opens, the draft survives, typing filters the list, and picking a
   file inserts `@path`.
2. Open a chat with no connected folder: `+` is disabled and its tooltip says
   why.
3. `bun run tauri dev`: the app starts once and stays up — no rebuild triggered
   by the sidecars the dev command just staged.
