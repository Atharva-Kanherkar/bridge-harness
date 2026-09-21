# fix/attention-notification-ui — Test Contract

## Functional Behavior

Improve attention notification UX so humans understand *what* Bridge needs,
using the same glass toast system as CI (`GithubToasts`) / update toasts.

- **In-app attention toasts**: every attention event (`needs-you` /
  `turn-completed` from `diffAttentionEvents`) enqueues a glass toast card
  (`u-glass-popover`, same layout/tokens as `GithubToasts`).
  - Headline names the kind clearly (`Bridge needs you`, `Turn completed`,
    or `Turn failed` when status is `failed`).
  - Detail line includes chat name, harness label, and a short reason
    (waiting for input / approval vs finished / failed).
  - Clicking the body opens that session; X dismisses; auto-dismiss after TTL.
  - Stack caps at the last 3 cards (same pattern as CI toasts).
- **OS notifications** (unfocused + Tauri only, existing focus gate kept):
  use the same headline + detail strings as the toast — no more bare
  `"Title a is waiting for your input"` without harness/context.
- Existing attention *diff* rules are unchanged (hidden sessions excluded,
  startup snapshot silent, permission-promise sharing intact).
- Focused Bridge still suppresses OS banners; in-app toasts still appear so a
  background chat that needs you is visible without leaving the current chat.

## Unit Tests

- `src/attentionCopy.test.ts`
  - `needs-you` → headline `Bridge needs you`, detail includes chat name +
    harness + waiting reason.
  - `turn-completed` with `ready` → `Turn completed` + finished detail.
  - `turn-completed` with `failed` → `Turn failed` + failed detail.
  - Falls back to `label` when `title` is empty; harness id is humanized.
- `src/components/AttentionToasts.test.tsx`
  - Renders headline + detail; open on body click; dismiss on X; auto-TTL;
    renders nothing when empty.

## Integration / Functional Tests

- `src/App.attention.test.tsx`
  - On `working` → `waiting`, invokes `notifyAttention` with enriched
    headline/detail (not the old generic body-only strings).
  - Same transition also leaves an `AttentionToasts` card in the DOM with
    that detail text.
  - Opening the toast body selects that session (or calls `openSession`).

## Smoke Tests

- Attention + App vitest suites green.
- `bunx tsc -b` green for the frontend change set.

## E2E Tests

N/A — no automated macOS notification harness. Screenshots of the in-app
toast variants are attached to the PR.

## Manual / cURL Tests

1. `bun run tauri dev`, blur Bridge, finish a turn → OS banner uses enriched
   copy; return to Bridge → glass toast still (or already) visible.
2. Background chat enters `waiting` while another chat is focused → glass
   toast appears; click opens the waiting chat.
