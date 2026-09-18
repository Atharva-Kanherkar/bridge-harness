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

let permissionRequested = false;

async function ensureNotificationPermission(): Promise<boolean> {
  const { isPermissionGranted, requestPermission } = await import("@tauri-apps/plugin-notification");
  if (await isPermissionGranted()) return true;
  if (permissionRequested) return false;
  permissionRequested = true;
  return (await requestPermission()) === "granted";
}

/**
 * Fires a native OS notification, but only when Bridge is not the focused,
 * foreground app — a focused Bridge window means the human is already
 * looking at whatever needs their attention, so a banner on top would be
 * noise rather than help. Outside Tauri (mock/dev/test) this is a no-op.
 */
export async function notifyAttention(title: string, body: string): Promise<void> {
  if (isBridgeFocused() || !isTauri()) return;
  if (!(await ensureNotificationPermission())) return;
  const { sendNotification } = await import("@tauri-apps/plugin-notification");
  sendNotification({ title, body });
}
