# fix/chat-smoothness-p1 — Test Contract

Covers the P1 findings of the chat smoothness audit (F1–F4) and three P2 fixes
that sit in the same code: F6 (pending delivery label and every image), F7
(worker `SteerComposer`), F11 (reduced motion for JS smooth scroll). F5, F8–F10,
F12–F14 and U1–U7 are out of scope and stay open on the audit issue.

## Functional Behavior

### F1 — drafts belong to their chat
- The chat composer's text and pasted images are kept per session. Switching
  A → B stores A's draft and shows B's (empty if B never had one).
- A → B → A shows A's draft again, text and images.
- Sending in B never carries A's text or images.
- A flow that opens a chat with a prepared draft (Work task "start") puts the
  draft in that chat, not in the chat the user was on.
- A shortcut that opens another chat (`$codex …`) consumes the draft of the chat
  it was typed in; returning there shows an empty composer.
- The welcome (hero) composer keeps its own state and is untouched.

### F2 — a failed send never overwrites or loses the user's words
- Failure while the chat is still on screen and the composer is empty: the sent
  text and images come back.
- Failure after the user typed something new: the failed text is put back
  first, then a blank line, then what they typed (`prev\n\nnext`). Images are
  merged, the failed send's first.
- Failure after the user switched to another chat: the text and images go into
  the origin chat's draft and appear when the user returns. The open chat's
  composer is untouched.
- A local command (`/usage`, `/compact`, …) whose `submitInput` succeeded but
  whose follow-up `reload()` fails leaves the composer empty (no duplicate
  send on retry) and still shows the error.
- An agent shortcut (`#agent …`) whose dispatch succeeded but whose `reload()`
  fails does not put the text back either.
- The sign-in retry path (`sendPrompt(forcedText, forcedAttachments)`) clears
  the composer only when it still holds exactly the retried text; other typing
  survives.

### F3 — a repeated message shows its bubble at once
- A pending send records, at send time, the newest timestamp of a user row in
  the same session with the same trimmed text (`after`). Only a user row with
  that text stamped strictly later answers it.
- Rows with no parseable timestamp answer any pending send with the same text
  (the old behavior; keeps stamp-less fixtures working).
- Each row answers at most one pending send, in send order.
- Sending `continue` when an earlier user turn says `continue` shows the
  optimistic bubble immediately; after delivery exactly one new row exists.
- Preparation that rewrites the text recomputes `after` for the new text.
- App reconcile and the `AgentConversation` render guard use the same rule.
- Known limit, documented in code: two identical sends in flight at once can
  hide the second bubble when the first lands (no stranding either way).

### F6 — the bubble says how it was delivered and shows every image
- A pending send that the backend steered shows "Steering"; one it queued shows
  "Queued — sent when this step finishes". A new turn shows no label.
- Every attached image of a pending send renders in that send's bubble, not
  only the first, and not on another send's bubble.

### F4 — old active pins are always candidates
- Eligible candidates are read with `status='active'` in SQL, ranked in SQL
  exactly like the packet ranks them (explicit first, confidence desc, newest
  first, id desc). Non-active churn cannot push an active pin out.
- Proposed/rejected/superseded/expired rows are still audited by code from a
  separate capped query (newest 200).
- `candidate_count` equals the number of active rows read.
- If the active set exceeds the candidate cap, the audit carries one
  `candidate_window_truncated` exclusion.

### F7 — worker `SteerComposer`
- Remounts per worker (`key={session.id}`): draft, busy, failure never leak to
  another worker.
- On success the draft is cleared only if it still equals the sent text.
- Enter during IME composition does not send.
- The failure message is `role="alert"`.

### F11 — reduced motion
- `scrollBehavior()` returns `"auto"` under `prefers-reduced-motion: reduce`,
  `"smooth"` otherwise, and `"smooth"` when `matchMedia` is unavailable.
- All three JS `scrollIntoView({ behavior: "smooth" })` sites use it.

## Unit Tests

- `src/composerDrafts.test.ts`
  - `mergeFailedSend` into empty / whitespace / non-empty composer; image merge
    order and id de-duplication; empty failed text keeps current text.
  - store save/take: empty drafts are not stored; take removes.
- `src/conversation.test.ts`
  - `userTurnWatermark` picks the newest matching stamp across projections and
    ignores other text, assistant rows and unparseable stamps.
  - `answeredPending`: repeated text with `after` is not answered by the old
    row and is answered by a newer one; one row answers one send; stamp-less
    rows keep legacy matching; trimmed text.
  - `undeliveredPending` (updated signature with durable rows) keeps all
    existing cases and adds the repeated-text case for live and durable.
- `src/motion.test.ts`: `scrollBehavior` with reduce / no-preference / no
  `matchMedia`.
- `src/components/SteerComposer.test.tsx`: rerender with a new `sessionId`
  resets draft; editing during an in-flight steer survives success; Enter with
  `isComposing` does not send; failure is `role="alert"`.
- `src/components/AgentConversation.motion.test.tsx`: existing pending cases
  with the object shape; repeated text with `after` renders the bubble; delivery
  label renders; every image renders.
- Rust `memory_packet::tests`:
  - `an_old_active_pin_survives_newer_churn` — 1 old explicit active pin plus
    250 newer superseded/rejected rows: packet contains the pin, audit
    `candidate_count == 1`.
  - `every_exclusion_class_is_audited_by_code` still passes unchanged.
  - `a_truncated_candidate_window_is_audited` — more active rows than the cap
    emits `candidate_window_truncated` (cap made testable via a const).
  - existing ranking tests unchanged.

## Integration / Functional Tests

App-level (`src/App.drafts.test.tsx`, jsdom, mock api):
- F1: type `x` in A, open B → composer empty; send `y` in B →
  `submitInput(B, "y", [])`; back to A → composer `x`.
- F1: pasted image in A does not show in B and returns with A.
- F2: deferred `prepareTurn` rejection while the user types `next` →
  composer `prev\n\nnext`.
- F2: switch to B before the rejection → B composer untouched; back to A →
  composer `prev`.
- F2: `/usage` with `reload` rejecting after `submitInput` resolved → composer
  empty, error shown, `submitInput` called once.
- F3: in the idle demo session, send text equal to an earlier user turn while
  `prepareTurn` is held → the bubble is visible; after delivery exactly one
  more row with that text exists.
- Existing `App.instantSend`, `App.composer`, `App.harnessShortcut`,
  `App.agentShortcuts`, `App.browserSelection`, `AsideChat` suites stay green.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar + vitest + cargo).
- `bun run dev` boots; mock chat send works.

## E2E Tests

N/A — no E2E harness for the desktop app. Covered by App-level jsdom tests plus
a manual pass in `bun run dev` (mock data) below.

## Manual / cURL Tests

`bun run dev` → http://127.0.0.1:1420, drive with a browser:
1. Open chat A, type "draft-A", open chat B: composer empty. Back to A:
   "draft-A".
2. In an idle chat whose history has "Run the full suite before we land this.",
   send exactly that text: the bubble appears immediately and one new row lands.
3. Toggle OS "reduce motion" (or emulate via devtools) and use recall search
   jump: no smooth scroll.

No cURL surface (desktop app, no HTTP API change).
