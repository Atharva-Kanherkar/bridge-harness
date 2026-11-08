# Contract: one toast stack with one dismiss behaviour

Branch `feat/attention-toast-stack`, stacked on `feat/attention-policy`.
Implements the second part of #909, slice 5 of #904. Sounds and per-kind
settings are the next PR. Locked before implementation.

## What this changes

Attention, connector and CI toasts already share a top-right container, but
each kind draws its own card, runs its own timer and caps itself differently,
and the update toast sits alone at the bottom left. The CI and connector
timers also restart whenever App re-renders, because they depend on the
`onDismiss` closure App recreates on every render (its git poll re-renders
every few seconds), so those toasts can outlive their TTL indefinitely.

## Functional behavior

- One card shell, `ToastCard` in `src/components/ToastCard.tsx`: icon, a body
  button (headline, detail, accessible name "<headline> — <where it goes>"),
  an optional action area, and the dismiss X ("Dismiss notification"). It
  owns the timer: `durationMs` (`null` stays until dismissed), restarted only
  by a new `key` or `firedAt`, never by a new `onDismiss` identity.
- Attention, CI, connector and update toasts all render through it. Each keeps
  its icon, copy, durations (attention per the policy, CI 15 s, connector
  20 s, update none) and extras: the connector shimmer, settle and entrance
  stagger, the CI jump hint, and the update's Install and restart button and
  error line.
- The update toast joins the top-right stack, last, instead of the bottom-left
  corner.
- Each kind shows its newest 3 and counts the rest ("+N more waiting"); the
  CI stack, which had no cap, gets the same one.
- Everything else (what each card opens, the Inbox suppression of connector
  toasts) is unchanged.

Out of scope: sounds and per-kind settings (next PR), writing CI or connector
events to the attention ledger.

## Unit tests

- `ToastCard.test.tsx`: expires after `durationMs`; `null` never expires; a
  re-render with a new `onDismiss` does not restart the timer; a new
  `firedAt` does; the body and the X call their handlers.
- `GithubToasts.test.tsx` and `ConnectorToasts.test.tsx`: a parent
  re-rendering with a fresh `onDismiss` every few seconds still sees the toast
  expire on time (fails before this change); CI caps at 3 with "+N more
  waiting".
- `UpdateToast.test.tsx`: renders inside the stack's flow (no fixed
  bottom-left position), installs, and shows the error line on failure.
- Existing `AttentionToasts`, `ConnectorToasts` and `GithubToasts` tests keep
  passing.

## Integration / functional tests

- `App.attention.test.tsx` keeps passing (toasts, Deliver now, policy).

## Smoke tests

- `bunx vitest run src/components/*Toast*.test.tsx src/App.attention.test.tsx`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Mock mode in a browser: an attention toast, a connector toast and the update
toast together in the top-right stack, dark and light screenshots.
