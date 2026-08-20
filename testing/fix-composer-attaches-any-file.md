# fix/composer-attaches-any-file — Test Contract

Locked before implementation.

## The defect

`+` was wired to the workspace `@`-mention picker. That reused existing plumbing
but does not do what the control is for:

- it could only reach files **inside the chat's connected folder**, and
- it was **disabled entirely** in a chat with no folder — which is most chats.

What is actually wanted, and what every comparable tool does: `+` opens the
system file dialog and attaches **any file on the machine**, in **any chat**.

## Expected behavior

- `+` opens the native file dialog (multi-select) in every chat, whether or not
  a folder is connected. It is never disabled for lack of a workspace.
- Picked paths are appended to the draft as `@path` mentions. The draft is added
  to, never replaced or cleared.
- A path with spaces or Unicode is JSON-quoted so it stays addressable.
- The caret returns to the composer afterwards.
- At submit time the backend reads each mention and attaches its contents as
  untrusted, bounded, secret-sanitized context — **without a workspace**.

### Authority, deliberately split

Widening this must not widen what an *agent* can read. Two rules, chosen by the
shape of the path:

- **Absolute** (`/…`, `~/…`) → read directly. The user named that exact file,
  usually from their own file dialog. A file outside the project is not out of
  bounds for being outside the project.
- **Relative** (`docs/x.md`) → still resolved through the workspace directory
  capability, so `..` and symlink escapes stay refused. A half-specified path
  keeps the capability it was typed under.

Every mention comes from the **user's own message**; model output never reaches
this resolver, so no agent gains reach from this change. Per-file and per-turn
byte caps and secret sanitization apply identically to both kinds.

## Unit Tests

### Rust (`bridge-core::workspace_files`)

- `an_absolute_mention_resolves_from_anywhere_including_a_chat_with_no_folder` —
  the case the old resolver could not serve at all; also proves having a
  workspace does not confine the user to it.
- `an_absolute_mention_is_still_bounded_and_redacted` — an API key in a picked
  file is redacted; an oversized file is truncated.
- `only_regular_files_are_inlined` — a directory and a missing path resolve to
  nothing, so a turn cannot stall on a non-file.
- `a_tilde_mention_expands_to_the_users_home` — `@~/x` is what a person types.
- `a_relative_mention_still_cannot_escape_its_workspace` — the traversal refusal
  survives the widening.
- Existing `mention_context_rejects_traversal_and_redacts_secrets` and
  `mention_context_rejects_symlink_escape` still pass, now against `Some(root)`.

### Frontend

- `appendFileMention` — appends without disturbing the draft, handles spacing,
  handles an empty draft, and handles several files from one pick.
- `formatFileMention` — an absolute path with spaces is JSON-quoted.
- `ComposerPill` tests unchanged: `+` calls its handler, leaves a non-empty draft
  alone, and stays reachable while working.

## Smoke Tests

- `bun run build` — green.
- `bun run test` — green.

## Manual / Live Verification

Requires the desktop shell (the system dialog does not exist in a browser).

1. In a chat with **no folder connected**, type a partial message, click `+`,
   pick a file from anywhere (e.g. `~/Desktop`): the path appends as a mention,
   the draft survives, and the agent's reply reflects the file's contents.
2. Pick several files at once: each appends as its own mention.
3. Pick a file whose name contains spaces: it appends quoted and still resolves.
4. In a chat **with** a folder, `@docs/` autocomplete still works unchanged.
