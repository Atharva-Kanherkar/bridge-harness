// Linux notifications with Allow once / Decline / Open (src-tauri/src/notifications.rs).
//
// The webview already knows when a session starts waiting on you; on Linux it
// hands that to the shell, which posts a freedesktop notification with action
// buttons. A clicked action comes back here and resolves through
// `bridgeApi.resolveApproval`, the same call the in-app card makes.

export const NOTIFY_EVENT = "bridge-native-notify";
export const ACTION_EVENT = "bridge-notification-action";

export type NotificationAction =
  | { action: "allow-once" | "decline"; sessionId: string; eventId: number }
  | { action: "open"; sessionId: string; eventId: number | null };

type EventLike = { sequence: number; kind: string; data?: Record<string, unknown> | null };

/** The sequence of the newest approval in `events` that nothing has resolved yet. */
export function pendingApprovalEventId(events: readonly EventLike[]): number | null {
  const resolved = new Set<number>();
  for (const event of events) {
    if (event.kind !== "approval.resolved") continue;
    const id = Number(event.data?.requestEventId);
    if (Number.isFinite(id)) resolved.add(id);
  }
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.kind === "approval.requested" && event.sequence > 0 && !resolved.has(event.sequence)) return event.sequence;
  }
  return null;
}

export function parseNotificationAction(raw: unknown): NotificationAction | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  if (typeof value.sessionId !== "string" || !value.sessionId) return null;
  const eventId = typeof value.eventId === "number" && Number.isFinite(value.eventId) ? value.eventId : null;
  if (value.action === "open") return { action: "open", sessionId: value.sessionId, eventId };
  if ((value.action === "allow-once" || value.action === "decline") && eventId !== null) {
    return { action: value.action, sessionId: value.sessionId, eventId };
  }
  return null;
}
