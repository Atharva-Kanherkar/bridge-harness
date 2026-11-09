# Contract: macOS banners open the chat and answer approvals

Branch `feat/attention-macos-actions`, stacked on `feat/attention-wait`.
Implements the second half of #910, slice 6 of #904. Locked before
implementation. The person asked for this to ship even though it cannot be
run here: it needs a signed app bundle and the notification permission. The
code is written against Apple's documented API, and every part that can be
tested without a bundle is.

## What this changes

On macOS a Bridge banner carries no target and no actions: clicking it only
brings Bridge forward, and an approval has to be answered inside the app.
Linux already has Allow once / Decline / Open on the banner
(`src-tauri/src/notifications.rs`). This gives macOS the same.

## Functional behavior

Webview (`src/attention.ts`):

- In Tauri, every platform hands a banner to the shell over the existing
  `bridge-native-notify` event (`{ title, body, sessionId, approvalEventId }`),
  as Linux already does. The webview no longer calls the notification plugin
  or asks for its permission; the shell does.
- `App.tsx` listens for `bridge-notification-action` on macOS too, so a
  clicked action resolves through `bridgeApi.resolveApproval`, the same call
  the in-app card makes, and "Handled already" covers a second resolution.

Native (`src-tauri/bridge-menu-bar/swift/Notifications.swift`, the crate that
already builds Bridge's Swift):

- Usable only from an app bundle (`Bundle.main` has an identifier and is a
  `.app`); `UNUserNotificationCenter` cannot be used from a bare binary.
- Two categories: `bridge.approval` with Allow once (requires unlocking the
  Mac), Decline (destructive) and Open (brings Bridge forward), and
  `bridge.chat` with Open. A request with a chat and an approval id gets the
  first, a request with a chat the second, anything else neither.
- The chat and approval ids ride in `userInfo`. A banner for a chat uses the
  identifier `bridge.chat.<id>`, so a newer one replaces the older.
- No sound on the banner: Bridge plays its own (#909).
- Permission is asked on the first banner, not at launch; a denial posts
  nothing.
- The delegate shows banners while Bridge is frontmost too (Bridge already
  decides when to post), and turns a response into
  `{ action, sessionId?, approvalEventId? }`: the default click is `default`,
  a dismissal is nothing.

Shell (`src-tauri/src/notifications.rs`):

- macOS: installs the native center at startup. On `bridge-native-notify` it
  posts natively; without a bundle it falls back to the notification plugin
  from Rust (title and body only), so `tauri dev` still shows banners.
- A native click goes through the same `action_taken` as Linux: an action that
  needs an approval id it does not carry resolves nothing. `open` (and the
  default click) shows and focuses the main window first. Then it emits
  `bridge-notification-action`.

Out of scope: notification grouping, provisional authorization, Windows.

## Unit tests

- Swift (`bun run test:menu-bar`): the category for each kind of request and
  its action ids and options; `userInfo` and the per-chat identifier; the
  response mapping (default, dismiss, each action, the ids carried); a
  malformed request builds nothing.
- Rust: a native click payload maps through `action_taken` exactly as a Linux
  click (approval, open, default, a forged allow without an id).
- `attention.test.ts`: in Tauri, macOS and Linux both emit
  `bridge-native-notify` with the ids; nothing while focused; outside Tauri
  the web branch is unchanged.

## Integration / functional tests

N/A here: the delegate and authorization run only in a signed bundle.

## Smoke tests

- `bun run test:menu-bar`, `cargo test -p bridge-deck notifications`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual (reviewer, release bundle)

1. A background chat asks for approval: the banner has Allow once, Decline
   and Open. Allow once (after unlocking) resolves it; the chat continues.
2. Click a banner's body: Bridge comes forward on that chat.
3. Answer the same approval in the app first, then on the banner: "Handled
   already".
