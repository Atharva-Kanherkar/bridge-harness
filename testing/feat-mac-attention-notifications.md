# feat/mac-attention-notifications — Test Contract

## Functional Behavior

Bridge fires a native macOS notification when it needs the human's attention and
the human is not currently looking at Bridge. Attention is derived by watching
`Session.status` transitions across every session in `BridgeState.sessions`
(refreshed on `onStateChanged`), not just the currently open chat — a background
session that starts waiting on the user must still surface a notification.

- **Turn completed**: a session's status bucket (`statusBucket` in
  `src/components/sidebarChats.ts`) leaves `"active"` (`working`, `starting`,
  `resuming`, `restored`, `checkpointing`, `warm`) and lands anywhere other than
  `"waiting"` (i.e. `ready`, `completed`, `cancelled`, `failed`, `idle`, `stopped`).
  This covers `turn.completed` and `turn.failed` (which the transcript codec
  already normalizes to `turn.completed`), since both retire the session's
  active-turn state in `live_turn.rs`.
- **Needs a human**: a session's status bucket transitions *into* `"waiting"`
  from any other bucket. This covers `permission.requested`,
  `question.requested` / `interaction.requested`, and worker
  `awaitingApproval` / `delegation.blocked` states, all of which the backend
  already surfaces as `SessionStatus: "waiting"` (the sidebar's existing "needs
  you" bucket).
- A transition into `"waiting"` is always reported as "needs you", never as
  "turn completed", even though it also leaves the active bucket.
- No notification fires for: the initial state load (no prior snapshot to diff
  against), a status that repeats the previous poll, or a transition between
  two non-active, non-waiting buckets (e.g. `ready` → `idle`).
- **Hidden sessions excluded**: `isHiddenSession` kinds (`briefing`,
  `suggestion`, `extraction`, `outcome_evaluation`, `consolidation`) never
  produce attention events — background maintenance must not look like a
  visible chat finishing or waiting.
- **Suppression**: no OS notification is dispatched while Bridge's main window
  is focused (`document.hasFocus()` in the main window's webview, tracked via
  `focus`/`blur` listeners). The attention-diffing logic still runs so no
  transition is missed if focus changes mid-session, but delivery is gated on
  the focus flag at fire time.
- Outside Tauri (mock/dev/test), notification delivery is a no-op — only the
  pure diffing logic is exercised.
- The OS notification's title/body includes the session's display name
  (`chatName()`: `title || label`) so the user knows which chat needs them.
- **Permission sharing**: concurrent `notifyAttention` calls while permission
  is still being requested share one in-flight `requestPermission` promise so
  a grant delivers every pending notification, not only the first.

## Unit Tests

- `src/attentionEvents.test.ts`
  - `diffAttentionEvents` returns `[]` when `previous` is `undefined` (startup).
  - `diffAttentionEvents` returns `[]` when no session's status changed.
  - `working` → `waiting` yields one `needs-you` event for that session.
  - `working` → `ready` yields one `turn-completed` event for that session.
  - `working` → `failed` yields one `turn-completed` event (turn failure still
    completes the turn).
  - `waiting` → `working` (human replied) yields no event.
  - `ready` → `idle`-equivalent bucket-preserving change yields no event when
    the bucket is unchanged, even if the raw status string differs (N/A today
    since every status maps to a distinct bucket except the `active` set —
    covered by an `active`-bucket-internal transition, e.g. `starting` →
    `working`, yielding no event).
  - A session present in `next` but absent from `previous` (newly created)
    yields no event.
  - Multiple sessions changing in the same poll each produce their own event,
    in `next` order.
  - A hidden-kind session (`briefing` / `suggestion` / `extraction` /
    `outcome_evaluation` / `consolidation`) transitioning active→idle or into
    waiting yields no event.
- `src/attention.test.ts`
  - `notifyAttention` does not call the notification plugin when
    `document.hasFocus()` is `true`.
  - `notifyAttention` does not call the notification plugin outside Tauri
    (`__TAURI_INTERNALS__` absent), regardless of focus.
  - `notifyAttention` calls `sendNotification` with the given title/body when
    unfocused, inside Tauri, and permission is already granted.
  - `notifyAttention` requests permission exactly once when not yet granted,
    and skips sending if the request is denied.
  - Concurrent `notifyAttention` calls while permission is outstanding share
    the same in-flight request; once granted, every caller sends.
  - The focus tracker flips to `false` on a `window` `blur` event and back to
    `true` on `focus`, independent of any Tauri API.

## Integration / Functional Tests

- `src/App.attention.test.tsx` (or equivalent colocated test exercising the
  effect wired in `App.tsx`): given a sequence of two `BridgeState` snapshots
  where one session's status moves from `working` to `waiting`, the attention
  effect invokes `notifyAttention` once with that session's `chatName()` in the
  body. A second render with an unchanged snapshot does not invoke it again.
- N/A for a Rust-side integration test: this change adds no new Rust business
  logic, only plugin registration (`tauri_plugin_notification::init()`) and a
  capability permission entry. Correctness there is verified by `cargo check
  --workspace` (the plugin/capability schema is validated at build time) and
  the manual test below.

## Smoke Tests

- `bun run check` passes (tsc -b + cargo check --workspace).
- `bun run test` passes (sidecar node:test + vitest run + cargo test
  --workspace).
- The app still boots and the existing meter tray / main window behavior is
  unchanged (no regression in `meter_tray.rs`, which this feature does not
  touch).

## E2E Tests

N/A — there is no automated harness in this repo for asserting a real macOS
`UNUserNotificationCenter` banner appeared. Covered instead by the manual test
below.

## Manual / cURL Tests

1. Run `bun run tauri dev` (or a release bundle per
   `docs/bridge-tauri-dev-restart-loop` guidance) and grant the notification
   permission prompt when macOS asks.
2. Start a chat, `cmd+tab` away to another app (or click off Bridge) so it is
   no longer the focused app, then let the turn finish. Expect a native
   notification titled to reflect a completed turn, naming the chat.
3. From a worker/session state that requires approval (e.g. a permission
   request), switch focus away from Bridge before the request lands. Expect a
   native "Bridge needs you" notification naming the chat.
4. Repeat steps 2–3 while Bridge is the focused, foreground app. Expect no OS
   notification in either case.
5. Confirm the menu-bar meter panel opening (which intentionally does not
   focus the app, per `meter_tray.rs`) does not, by itself, count as "focused"
   — a turn completing while only the meter panel is open should still
   notify.
