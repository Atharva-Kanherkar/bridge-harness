# Contract: one banner owner across clients

Branch `feat/attention-banner-owner`, stacked on `feat/attention-sounds`.
Implements the first half of #910, slice 6 of #904: who posts OS banners.
macOS banner actions (click-to-chat, Allow once / Decline) are the second
half, in their own PR. Locked before implementation.

## What this changes

Every client decides on its own whether to post an OS banner, so a desktop
app and a second client (another desktop window on the same daemon, or a
future web banner channel) can both post one for the same event. Core now
holds a short presence lease per client and names one banner owner.

## Functional behavior

Core (`bridge-core/src/attention_presence.rs`), in memory on `BridgeCore`:

- `report(client_id, kind, focused, gone, now)` records
  or renews a client's lease and returns whether that client owns banners.
  `kind` is `desktop` or `web`. `gone` removes the client at once (a window
  closing).
- A lease expires 30 s after its last report. Expired clients are dropped
  before every election.
- Owner: if any live client is focused, the one that became focused most
  recently. Otherwise the live desktop client that was focused most recently,
  and among desktop clients never focused, the earliest to register. A web
  client that is not focused never owns banners (it has no banner channel
  yet, #914).
- The owner is stable between reports: renewals alone never move it.
- Nothing is persisted; a daemon restart starts empty, and the first report
  elects again.

Protocol 1.27: `attention/report_presence { clientId, clientKind, focused,
gone? } -> { bannerOwner }`. Strict params. A 1.26 client
still pairs with a 1.27 daemon (it never reports, and keeps posting banners
as before). A 1.27 client against an older daemon gets `method_not_found`,
which the client treats as owning banners, so nothing goes silent.

Desktop:

- `useBannerOwner(focused)` reports on mount, on every change of focus, and
  every 10 s; on `pagehide` it reports
  `gone`. It returns the last answer, `true` before the first one and after
  a failed one.
- Renewal every 10 s against a 30 s lease puts a crashed owner's handover
  within 40 s, inside the issue's 45 s bound. (The issue sketched 20 s and
  45 s; that bound would allow 65 s.)
- `decide` gains `presence.bannerOwner`: no banner unless this client owns
  them. Toasts and sounds are unchanged; every client still shows its own
  toasts while focused.
- Client id: one random id per app load. Kind: `desktop` in Tauri, `web`
  otherwise. Mock mode always owns.

Out of scope: macOS banner actions (next PR), a web banner channel (#914),
using remote presence for toast suppression.

## Unit tests (bridge-core)

- `attention_presence::tests::a_lone_desktop_client_owns_banners`
- `attention_presence::tests::the_focused_client_owns_and_hands_back_on_blur`
  (a focused web client takes ownership, so the desktop posts nothing; on
  blur the desktop owns again)
- `attention_presence::tests::renewals_do_not_move_the_owner`
- `attention_presence::tests::an_expired_or_gone_owner_hands_over`
- `attention_presence::tests::an_unfocused_web_client_never_owns`

## Integration / functional tests

- bridged dispatch: two clients report; only one gets `bannerOwner: true`;
  unknown fields are refused.
- Protocol gates, and a handshake test naming the 1.27 boundary both ways.

## Frontend tests

- `attentionPolicy.test.ts`: a non-owner gets no banner; toast and sound are
  unchanged.
- `useBannerOwner.test.tsx`: reports on mount and on a focus or chat change;
  renews on the interval; reports `gone` on `pagehide`; a failure leaves it
  owning.
- `App.attention.test.tsx`: when the core says another client owns banners,
  a background row shows its toast and never calls the banner path.

## Smoke tests

- `cargo test -p bridge-core attention_presence`, `cargo test -p bridge-protocol`,
  `cargo test -p bridged dispatch`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Live daemon from this branch: two `bridge exec --method attention/report_presence`
calls with different client ids show one owner; a focused report from the
second takes it; `gone` hands it back.

## Amendment (before the protocol change)

The issue sketched a `visibleSessionId` in the report, for core to suppress
notices about a chat that is on screen in another client. Nothing in this
slice would read it, so it is left out rather than stored unused; the slice
that wires cross-client suppression adds it.

