# Contract: the unread count in the native menu bar

Branch `feat/attention-menu-bar`, stacked on `feat/attention-unread`.
Implements the second half of #908, slice 4 of #904. Locked before
implementation.

## What this changes

The previous PR put unread chats on the sidebar and the Dock icon. People who
keep Bridge's menu bar item on (Settings → Menu Bar) also see it there: a dot
on the status item while anything is unread, and a row at the top of its menu
that names the count and opens Bridge.

## Functional behavior

Shell (`src-tauri/src/menu_bar/mod.rs`):

- The presentation snapshot gains `attention: { total, needsYou } | null`,
  read from `attention/unread_summary` through the host's existing `call()`
  (daemon proxy or embedded core), on every pass of the host loop.
- A failed read leaves `attention` as it was. The usage parts of the
  snapshot behave exactly as before.
- The host also wakes on the webview's `bridge-unread-badge` event, so a
  change reaches the menu bar without waiting for the 30 s tick. The read
  never starts a provider request or a usage refresh.

Native (`src-tauri/bridge-menu-bar/swift/`):

- `Presentation.attention` is optional: a snapshot without it decodes as
  before.
- `attentionLabel(total, needsYou)`:
  - 0 total: no row.
  - needsYou only: "1 chat needs you", "2 chats need you".
  - nothing needs you: "1 unread chat", "3 unread chats".
  - both: "1 chat needs you · 3 unread".
- The menu's first item is that label with a bell symbol, followed by a
  separator. Choosing it opens Bridge (the existing Open Bridge action).
  Both are hidden when nothing is unread. Like the rest of the menu, the
  structure changes only while the menu is closed; the title updates in
  place.
- The status icon gets a small dot in its top-right corner while anything is
  unread, drawn into the template image so it follows the menu bar's
  appearance. This applies to every icon style and to per-provider items.
- The status item's tooltip and VoiceOver title end with the label
  (", 2 chats need you") while anything is unread.

Out of scope: per-chat rows in the menu, notification settings (#909).

## Unit tests

- Swift (`tests/main.swift`, run by `bun run test:menu-bar`):
  - `attentionLabel` for each case above.
  - The shared fixture decodes `attention`, and a snapshot without it still
    decodes.
  - `MenuStatus`/accessibility: the label is appended only when unread.
  - The dotted icon differs from the plain one, and stays a template image.
- Rust shell: the snapshot serializes `attention` as
  `{"total":…,"needsYou":…}` and matches the shared fixture.
- `bridge-protocol`: the fixture test still passes with the new key.

## Integration / functional tests

- Embedded host: `call()` answers `UnreadSummary` from core rather than
  "Unsupported menu request".

## Smoke tests

- `bun run test:menu-bar`
- `cargo test -p bridge-deck menu_bar`, `cargo test -p bridge-protocol`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

The status item belongs to the running app, and a second release bundle next
to the daily one risks its data directory. Synthetic renders of the dotted
icon (light and dark) from the Swift test harness go in the PR. Running it
live is left as a reviewer step: enable the menu bar, finish a turn in a
background chat, and check the dot and the row, then open the chat and check
both clear.

## Review amendments (review 2)

Locked before the fix; it gets a regression test that fails without it.

1. **The menu wakes when what needs you changes, not only the total.** The
   app sends `bridge-unread-badge` whenever `total` or `needsYou` changes, so
   a chat going from finished to needs-you (total still 1) reaches the menu
   at once instead of on the next 30 s pass. The Dock badge just sets the
   same count again. Test (`App.attention.test.tsx`): `setUnreadBadge` is
   called again when `needsYou` changes at the same total.
