// OS notifications for the web client, through the browser's Notification API.
// The desktop app posts its own through the shell; a browser tab opened with
// a daemon link posts these, when core names it the banner owner.

/** App listens for this to open the chat a clicked notification was about. */
export const WEB_NOTIFICATION_OPEN = "bridge:web-notification-open";

let permissionRequest: Promise<NotificationPermission> | null = null;

/** Whether this tab can post notifications now, or could after asking once. */
export function webCanPostBanners(): boolean {
  return typeof Notification !== "undefined" && Notification.permission !== "denied";
}

/**
 * Post one notification. Asks for permission on the first one it needs, once
 * per load however many arrive meanwhile; a denial is final and never asks
 * again. The chat id is the tag, so a newer notice for a chat replaces the
 * older one. Clicking focuses the tab and asks App to open the chat.
 */
export async function notifyInBrowser(title: string, body: string, sessionId?: string): Promise<void> {
  if (typeof Notification === "undefined") return;
  let permission = Notification.permission;
  if (permission === "default") {
    permissionRequest ??= Notification.requestPermission();
    permission = await permissionRequest;
  }
  if (permission !== "granted") return;
  const notification = new Notification(title, { body, ...(sessionId ? { tag: sessionId } : {}) });
  notification.onclick = () => {
    window.focus();
    if (sessionId) window.dispatchEvent(new CustomEvent(WEB_NOTIFICATION_OPEN, { detail: { sessionId } }));
    notification.close();
  };
}

/** Tests only. */
export function resetWebNotify(): void {
  permissionRequest = null;
}
