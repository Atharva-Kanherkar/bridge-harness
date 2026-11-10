# Contract: browser notifications for the web client

Branch `feat/attention-web-banners`, stacked on `feat/attention-banner-owner`.
Implements the web half of #914, slice 10 of #904. The terminal half waits
for the TUI (#611), which does not exist yet. Locked before implementation.

## What this changes

The web client (a browser tab opened with a daemon link) shows in-app toasts
but never an OS notification, and core never names it the banner owner while
it is unfocused. With only the web client open and the tab in the
background, a request goes unseen.

## Functional behavior

Core and protocol (1.28):

- `attention/report_presence` gains an optional `canPostBanners`. When it is
  absent, a desktop client can and a web client cannot, which is exactly the
  1.27 behavior.
- Election: a focused client still owns. Otherwise, among live clients that
  can post banners, the one focused most recently owns, and among those never
  focused, the earliest to register. A client that cannot post banners never
  owns while unfocused.
- A 1.27 client still pairs with a 1.28 daemon.

Web client (`src/attention.ts`, `src/useBannerOwner.ts`):

- It reports `canPostBanners: true` unless the browser has no `Notification`
  API or the person denied it.
- `notifyAttention` in a web client (not Tauri, not mock mode):
  - permission `granted`: posts `new Notification(title, { body, tag: sessionId })`;
  - permission `default`: asks once, on the first banner it needs, and posts
    if granted. One prompt per load, however many banners arrive meanwhile;
  - permission `denied`: posts nothing and never asks again.
- Clicking the notification focuses the tab and opens the chat.
- The same gates as today: only when Bridge is not focused, only when the
  policy says so, and only from the banner owner.

Out of scope: service-worker push while no tab is open, the TUI channel,
approval actions on a browser notification (browsers give no reliable action
buttons).

## Unit tests

- `attention_presence::tests`:
  - an unfocused web client that can post banners owns when no desktop is
    there;
  - with a desktop and a capable web client, the one focused last owns;
  - a client that cannot post banners never owns while unfocused.
- bridged dispatch: `canPostBanners` round trip; unknown fields still refused.
- Handshake test naming the 1.28 boundary.
- `attention.test.ts` (jsdom, stubbed `Notification`):
  - granted posts with title, body and the session as tag;
  - default asks once for two banners and posts both if granted;
  - denied posts nothing and never asks;
  - a click focuses the window and asks App to open the chat;
  - nothing is posted while Bridge is focused.
- `useBannerOwner.test.tsx`: reports `canPostBanners` from the browser.

## Integration / functional tests

- `App.attention.test.tsx`: a web notification click opens that chat.

## Smoke tests

- `cargo test -p bridge-core attention_presence`, `cargo test -p bridge-protocol`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

The web client in a real browser against a live daemon from this branch
(`bun run web:link`): background the tab, raise a request, see the browser
notification, click it, land in the chat. Recorded in the PR if the browser
lets an automated profile grant notification permission; otherwise left as
a reviewer step.
