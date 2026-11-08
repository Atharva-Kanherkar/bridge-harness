# Contract: one notification policy for attention rows

Branch `feat/attention-policy`, stacked on `feat/attention-menu-bar`.
Implements the first part of #909, slice 5 of #904: the decision rules. The
unified toast stack and the sounds and settings are the next two PRs.
Locked before implementation.

## What this changes

Today every ledger row the desktop feed hands over becomes a toast, the chat
you are looking at included, and every toast lasts 16 s. One pure function
now decides what a row becomes, ported from herdr's `notification_policy.rs`.

## Functional behavior

`decide(item, presence)` in `src/attentionPolicy.ts`. `presence` is
`{ focused: boolean; onScreen: ReadonlySet<string> }`, where `onScreen` holds
the chats whose transcript is in view. It returns
`{ toast: boolean; banner: boolean; durationMs: number | null }` or `null`
for nothing.

- `null` when the row has no session, is seen or resolved, or has a kind
  this build does not show.
- `null` when Bridge is focused and the row's chat is on screen. The person
  is looking at it: no toast and no banner. (The unread effect from #929
  marks it seen.)
- Otherwise a toast. A banner only when Bridge is not focused (the OS banner
  gate that `notifyAttention` applies today, now decided here).
- Durations: `needsYou` and `failed` 8 s, `finished` 5 s. `escalation` has
  no timeout (`null`): its Deliver now button is its only remedy, so it stays
  until the person acts on it or dismisses it.
- The 1 s hold and re-check for `needsYou` stays in `useAttentionFeed`; the
  policy sees only rows that survived it.

App:

- `showAttention` asks `decide` with the live presence: the window focus from
  `useWindowFocused`, and the selected chat when the workspace shows a single
  chat (not the Mission Control grid, whose tiles are left for later).
- Queue: newest per chat wins as today. Up to 8 toasts are kept and the
  newest 3 shown, so dismissing one reveals the next still waiting. A toast's
  timer starts when it is shown.
- `AttentionToast` carries `durationMs`; `null` means it stays.

Out of scope: one stack for CI, connector and update toasts (next PR),
sounds and per-kind settings (the PR after), Mission Control tiles.

## Unit tests

- `attentionPolicy.test.ts`, table-driven:
  - seen, resolved, session-less and unknown kinds return `null`;
  - the on-screen chat in a focused window returns `null`, in an unfocused
    window a toast and a banner;
  - another chat in a focused window gets a toast and no banner;
  - durations per kind, and `null` for `escalation`.
- `AttentionToasts.test.tsx`: a toast expires after its own `durationMs`, one
  with `null` never expires, and only the newest 3 of the queue render.

## Integration / functional tests

- `App.attention.test.tsx`:
  - a row for the chat on screen in a focused window shows no toast and
    posts no banner;
  - a row for another chat shows a toast, and its banner call is skipped
    while focused;
  - the existing toast, Deliver now and approval-id tests keep passing.

## Smoke tests

- `bunx vitest run src/attentionPolicy.test.ts src/components/AttentionToasts.test.tsx src/App.attention.test.tsx`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Mock mode in a browser: open a chat, push a `finished` row for it (no toast),
then for another chat (a toast that leaves after 5 s). Screenshot of the
toast in dark and light.
