import type { AttentionItem } from "./types";

// What an attention row becomes, decided in one place. Ported from herdr's
// `notification_policy.rs`: nothing about the chat you are looking at, a
// banner only when Bridge is in the background, and a toast that lasts as
// long as its kind deserves. The 1 s hold and re-check for requests happens
// earlier, in `useAttentionFeed`; rows that reach here survived it.

/** Toasts kept waiting, newest per chat; the stack shows the newest few. */
export const TOAST_QUEUE_MAX = 8;

export type Presence = {
  /** Bridge's window is focused and visible. */
  focused: boolean;
  /** Chats whose transcript is in view. */
  onScreen: ReadonlySet<string>;
};

export type Decision = {
  toast: boolean;
  /** The OS notification. */
  banner: boolean;
  /** How long the toast stays; `null` until the person acts on it. */
  durationMs: number | null;
};

/** A request or a failure is worth a few more seconds than a finished turn. */
const DURATION_MS: Partial<Record<AttentionItem["kind"], number | null>> = {
  needsYou: 8_000,
  failed: 8_000,
  finished: 5_000,
  // Its Deliver now button is the only remedy, so it waits for the person.
  escalation: null,
};

export function decide(item: AttentionItem, presence: Presence): Decision | null {
  const sessionId = item.sessionId;
  if (!sessionId || item.seenAt || item.resolvedAt) return null;
  const durationMs = DURATION_MS[item.kind];
  if (durationMs === undefined) return null;
  // The person is looking at it.
  if (presence.focused && presence.onScreen.has(sessionId)) return null;
  return { toast: true, banner: !presence.focused, durationMs };
}
