import { isMacPlatform } from "./lib/platform";
import { NOTIFY_EVENT } from "./nativeNotify";

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Tracked independently of any Tauri API so it works the same in the mock/dev
// webview: a native window's webview reliably gets `focus`/`blur` when the OS
// window itself gains or loses key status, which is all "is Bridge the app
// the human is looking at" needs.
let focused = typeof document !== "undefined" ? document.hasFocus() : true;
if (typeof window !== "undefined") {
  window.addEventListener("focus", () => { focused = true; });
  window.addEventListener("blur", () => { focused = false; });
}

export function isBridgeFocused(): boolean {
  return focused;
}

// Shared across concurrent notifyAttention calls so the first OS prompt is
// awaited by every in-flight caller — treating an outstanding request as
// denial would drop every banner after the first when one snapshot emits
// multiple attention events before the user answers. Assigned synchronously
// so two callers cannot both miss and start duplicate prompts.
let permissionRequest: Promise<boolean> | null = null;
let notificationModule: Promise<typeof import("@tauri-apps/plugin-notification")> | null = null;

function loadNotification() {
  notificationModule ??= import("@tauri-apps/plugin-notification");
  return notificationModule;
}

function ensureNotificationPermission(): Promise<boolean> {
  if (!permissionRequest) {
    permissionRequest = (async () => {
      const { isPermissionGranted, requestPermission } = await loadNotification();
      if (await isPermissionGranted()) return true;
      return (await requestPermission()) === "granted";
    })();
  }
  return permissionRequest;
}

/**
 * Fires a native OS notification, but only when Bridge is not the focused,
 * foreground app — a focused Bridge window means the human is already
 * looking at whatever needs their attention, so a banner on top would be
 * noise rather than help. Outside Tauri (mock/dev/test) this is a no-op.
 */
export async function notifyAttention(
  title: string,
  body: string,
  context: { sessionId?: string; approvalEventId?: number | null } = {},
): Promise<void> {
  if (isBridgeFocused() || !isTauri()) return;
  // Linux: the shell posts it natively, with Allow once / Decline / Open.
  if (!isMacPlatform()) {
    const { emit } = await import("@tauri-apps/api/event");
    await emit(NOTIFY_EVENT, { title, body, sessionId: context.sessionId ?? null, approvalEventId: context.approvalEventId ?? null });
    return;
  }
  if (!(await ensureNotificationPermission())) return;
  const { sendNotification } = await loadNotification();
  sendNotification({ title, body });
}
