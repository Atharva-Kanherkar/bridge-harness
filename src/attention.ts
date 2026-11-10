import { NOTIFY_EVENT } from "./nativeNotify";
import { activeWebDaemon } from "./webDaemon";
import { notifyInBrowser } from "./webNotify";

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
  if (isBridgeFocused()) return;
  if (!isTauri()) {
    // A browser tab with a daemon link posts through the browser. Mock mode
    // (`bun run dev` with no daemon) posts nothing.
    if (activeWebDaemon()) await notifyInBrowser(title, body, context.sessionId);
    return;
  }
  // The shell posts it, with Allow once / Decline / Open where the platform
  // allows: notify-rust on Linux, Notification Center on macOS. It also asks
  // for the permission; a clicked action comes back as ACTION_EVENT.
  const { emit } = await import("@tauri-apps/api/event");
  await emit(NOTIFY_EVENT, { title, body, sessionId: context.sessionId ?? null, approvalEventId: context.approvalEventId ?? null });
}
