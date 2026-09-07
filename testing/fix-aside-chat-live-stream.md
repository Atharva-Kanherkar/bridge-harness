# fix/aside-chat-live-stream — Test Contract

Aside chats hang at the cold-start narration ("Waiting for … to answer…" /
"… is reading your message…") and never show a reply, and switching the
aside's model gives no feedback and surfaces failures nowhere the user can
see. Root causes locked by this contract:

1. **Browser/mock mode delivers no live agent events.** The mock fan-out was
   removed alongside legacy agent-event persistence, so outside Tauri
   `bridgeApi.onAgentEvent` never invokes its handler. Every surface that
   reads the global live stream — the aside panel above all, since its
   optimistic pending rows reconcile *only* against the live stream — waits
   forever. The mock must fan out appended events exactly like the daemon
   does.
2. **The aside model switch has no narration and no in-panel error.** The
   main chat shows "Switching to …" through the `modelSwitch` prop and
   surfaces failures; the aside called `updateChatModel` fire-and-forget,
   with errors routed to the main error banner behind the scrim.
3. **Escape tears down the whole aside while the model picker is open.**
   The picker had no Escape handling, so the panel's window-level Escape
   closed the aside instead of the popover.
4. **The aside composer always says "Queue" mid-turn**, even for harnesses
   that advertise `steering` (the main composer resolves the verb through
   `activeTurnAction`).

## Functional Behavior

- In browser/mock mode, sending a message in an aside shows the user's row,
  then the mock assistant reply, and the startup narration unmounts. No
  permanent "reading your message…" row.
- Events appended by any mock path (`startChat`, `sendTurn`, `submitInput`,
  `resolveApproval`) reach `onAgentEvent` subscribers as clones, with
  `sequence` already assigned.
- Switching the aside model shows the "Switching to {label}…" narration row
  inside the aside conversation while `updateChatModel` is in flight, and
  clears it after.
- A failed aside model switch renders its error message inside the aside
  panel (same slot as composer errors), not only in the main banner.
- Escape with the model picker open closes the picker only; a second Escape
  closes the aside. Escape without the picker open still closes the aside.
- With an adapter advertising `steering`, the aside's mid-turn submit
  affordance reads "Steer"; without it, "Queue".

## Unit Tests

- `src/api.boundary.test.ts` (or `api.test.ts`) — mock `onAgentEvent`
  registers the handler outside Tauri, delivers appended events, and the
  returned unsubscribe stops delivery.
- `src/components/AsideChat.test.tsx`:
  - failed model switch → error text rendered inside the dialog.
  - model switch in flight → `modelSwitch` prop reaches the conversation
    (narration label "Switching to Opus…" present).
  - Escape with the picker open closes the picker, not the panel;
    Escape without it still calls `onClose`.
  - adapter with `steering` capability → submit affordance is "Steer".
- `src/components/ChatModelControl.test.tsx` — Escape closes an open picker
  and stops the event from reaching window-level bubble listeners.

## Integration / Functional Tests

- Existing `AsideChat` suite stays green (digest-gated forest poll, paste
  chips, approvals, promote, scrim close).
- Existing `App` aside tests stay green (aside opens from `$harness`
  shortcut, model resolution).

## Smoke Tests

- `bun run dev` → open a demo session → `$claude ask something` → aside
  shows the mock reply within a second; narration collapses; model pill
  switches to Opus and the switch narration appears/clears.

## E2E Tests

N/A — no Tauri-side changes; the daemon/live path is unchanged.

## Manual / cURL Tests

- Manual browser walk of the smoke test above (screenshot before/after).
